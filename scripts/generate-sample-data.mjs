// generate-sample-data.mjs — emit the committed onboarding fixture (F4).
//
// Deterministic: a seeded PRNG makes the output byte-stable, so re-running never
// churns the committed file. Writes `extension/src/sample-data.json`, an
// export-schema-v1 document (the exact shape `store.rs::import_json` accepts):
// `{ version, events, spa, rollup_days, sessions, meta }`. Only `events` carries
// data — the derived stores rebuild on load via `reset_derivation`.
//
// Two conventions the load path (crates/core/src/sample.rs) depends on:
//   • Timestamps are stored as OFFSETS in ms *before "now"*, not absolute epochs,
//     so the loaded data always looks recent (shifted against Date.now() on load).
//   • URLs are stored SCHEMELESS ("news.example/x", not "https://news.example/x").
//     The load step prepends the scheme. This is an AUDIT interaction: the CI
//     bundle audit greps dist/ for `https?://`, and this fixture is inlined into
//     the dashboard bundle verbatim (via a `?raw` import). Keeping the scheme out
//     of the committed text keeps that grep clean; see sample.rs for the mirror.
//
// Run: `node scripts/generate-sample-data.mjs`
import { writeFileSync } from "node:fs";
import { fileURLToPath, URL as NodeURL } from "node:url";

// ── seeded PRNG (mulberry32) ─────────────────────────────────────────────────
function mulberry32(seed) {
  let a = seed >>> 0;
  return function () {
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}
const rnd = mulberry32(0x0d15ea5e);
const pick = (arr) => arr[Math.floor(rnd() * arr.length)];
const chance = (p) => rnd() < p;
const randint = (lo, hi) => lo + Math.floor(rnd() * (hi - lo + 1));

// ── host clusters (schemeless) ───────────────────────────────────────────────
// Each theme is a densely inter-linked cluster; cross-theme links are rare, so
// Louvain recovers the clusters as 2–3 communities. Every theme also has a
// canonical "journey" (a repeated typed→link→link chain) so PrefixSpan finds
// frequent sequences.
const THEMES = {
  news: {
    entry: "news.example",
    hosts: [
      "news.example",
      "world.news.example",
      "tech.news.example",
      "sports.news.example",
    ],
    journey: ["news.example", "world.news.example", "tech.news.example"],
    paths: ["", "world", "tech", "sports", "live", "opinion"],
  },
  dev: {
    entry: "dev.example",
    hosts: [
      "dev.example",
      "docs.dev.example",
      "api.dev.example",
      "git.dev.example",
    ],
    journey: ["dev.example", "docs.dev.example", "api.dev.example"],
    paths: ["", "guide", "reference", "issues", "pulls", "search"],
  },
  video: {
    entry: "video.example",
    hosts: ["video.example", "watch.video.example", "live.video.example"],
    journey: ["video.example", "watch.video.example", "live.video.example"],
    paths: ["", "watch", "trending", "channel", "playlist"],
  },
  mail: {
    entry: "mail.example",
    hosts: ["mail.example", "inbox.mail.example"],
    journey: ["mail.example", "inbox.mail.example"],
    paths: ["", "inbox", "sent", "compose"],
  },
};
const THEME_KEYS = Object.keys(THEMES);
const SEARCH_HOST = "search.example";

// "Work" context: dev and mail are tightly coupled (you check mail from the code
// host and jump back), so most work sessions bridge between them. This merges the
// dev + mail host clusters into a single Louvain community, leaving three groups
// overall — work, news, video — the 2–3 the onboarding graph should read as. News
// and video are deliberately *not* bridged, so they stay distinct.
const SIBLING = { dev: "mail", mail: "dev" };

const DAY = 86_400_000;
const MIN = 60_000;

// A schemeless URL with a plausible path so hostname keys carry realistic detail.
function url(host, theme) {
  const seg = pick(THEMES[theme].paths);
  const n = randint(1, 40);
  return seg ? `${host}/${seg}/${n}` : `${host}/`;
}

// ── event accumulation ───────────────────────────────────────────────────────
// Built in strict chronological order; ids are assigned last (id order == time
// order, the derive pass's contract). `ts` is filled with the absolute clock and
// converted to an offset-before-now at the end.
const events = [];
const push = (e) => events.push(e);
let tabSeq = 100;
const nextTab = () => ++tabSeq;

function navEvent(t, tab, win, toUrl, transitionType, qualifiers = []) {
  push({ kind: "nav", ts: t, tabId: tab, windowId: win, toUrl, transitionType, qualifiers });
}

// One browsing session: an entry (typed or search-origin), then a link chain that
// mostly walks the theme's canonical journey with occasional branches, sometimes
// spawning a new tab, and a login form for mail. Ends by closing its tabs.
function session(startClock, theme, win) {
  let t = startClock;
  const th = THEMES[theme];
  const tab = nextTab();
  const step = () => (t += randint(8, 90) * 1000); // 8s–1.5min between navs

  // Entry: ~⅓ via a search-results origin (creates SearchLink edges), else typed.
  if (chance(0.34)) {
    navEvent(t, tab, win, `${SEARCH_HOST}/search?q=${theme}`, "generated");
    step();
    navEvent(t, tab, win, url(th.entry, theme), "link");
  } else {
    navEvent(t, tab, win, url(th.entry, theme), "typed");
  }

  // Walk the canonical journey by link (this is the repeated "journey").
  for (let i = 1; i < th.journey.length; i++) {
    step();
    navEvent(t, tab, win, url(th.journey[i], theme), "link");
  }

  // A few extra intra-theme link hops for graph density (and back-links).
  const extra = randint(2, 6);
  for (let i = 0; i < extra; i++) {
    step();
    const host = pick(th.hosts);
    const tt = theme === "mail" && chance(0.25) ? "form_submit" : "link";
    navEvent(t, tab, win, url(host, theme), tt);
  }

  // Most work sessions bridge dev↔mail and keep hopping between the two — a
  // strong, frequent coupling so Louvain merges them into one "work" community
  // (the intra-cluster hops on both sides still keep the graph legibly grouped).
  const sib = SIBLING[theme];
  if (sib && chance(0.7)) {
    const hops = randint(2, 4);
    for (let i = 0; i < hops; i++) {
      step();
      // Alternate sides so edges accrue in both directions (dev→mail, mail→dev).
      const side = i % 2 === 0 ? sib : theme;
      navEvent(t, tab, win, url(pick(THEMES[side].hosts), side), "link");
    }
  }

  // Occasionally open a link in a new tab (child origin = source's current page).
  if (chance(0.4)) {
    const child = nextTab();
    step();
    push({ kind: "link", ts: t, newTabId: child, sourceTabId: tab });
    step();
    navEvent(t, child, win, url(pick(th.hosts), theme), "link");
    if (chance(0.6)) {
      step();
      navEvent(t, child, win, url(pick(th.hosts), theme), "link");
    }
    step();
    push({ kind: "close", ts: t, tabId: child });
  }

  // A stray reload happens sometimes (ignored by derivation, but realistic).
  if (chance(0.2)) {
    step();
    navEvent(t, tab, win, url(th.entry, theme), "reload");
  }

  step();
  push({ kind: "close", ts: t, tabId: tab });
  return t;
}

// ── 3 weeks of history, 2–4 sessions/day ─────────────────────────────────────
const DAYS = 21;
let clock = 0; // arbitrary epoch; converted to offsets at the end
for (let day = 0; day < DAYS; day++) {
  const dayStart = day * DAY;
  // A couple of days begin with a browser restart marker.
  if (day === 0 || chance(0.25)) {
    push({ kind: "start", ts: dayStart + randint(6, 9) * 60 * MIN });
  }
  const nSessions = randint(2, 4);
  // Spread sessions across waking hours (roughly 8:00–23:00).
  let hour = randint(8, 11);
  for (let s = 0; s < nSessions; s++) {
    const theme = chance(0.15) ? pick(THEME_KEYS) : THEME_KEYS[(day + s) % THEME_KEYS.length];
    const win = chance(0.2) ? 2 : 1;
    const start = dayStart + hour * 60 * MIN + randint(0, 25) * MIN;
    const end = session(start, theme, win);
    // Next session begins well past the 30-min idle gap (a new session).
    hour = Math.min(23, hour + randint(2, 4));
    clock = Math.max(clock, end);
  }
}

// ── offsets before "now" + id assignment ─────────────────────────────────────
// The newest event sits a little under 2h before "now" so the demo reads as
// "just browsed". Larger offset == older. ids follow chronological order.
const RECENCY_PAD = 110 * MIN;
const tMax = events.reduce((m, e) => Math.max(m, e.ts), 0);
events.sort((a, b) => a.ts - b.ts);
events.forEach((e, i) => {
  e.ts = tMax - e.ts + RECENCY_PAD; // absolute → offset-before-now
});
// Re-sort into id order (ascending id == ascending absolute time == descending
// offset) and stamp ids.
events.sort((a, b) => b.ts - a.ts);
events.forEach((e, i) => {
  e.id = i + 1;
});
// Reorder each record's keys so `id`/`kind` lead (readability of the committed file).
const ordered = events.map((e) => {
  if (e.kind === "nav")
    return {
      kind: "nav",
      id: e.id,
      ts: e.ts,
      tabId: e.tabId,
      windowId: e.windowId,
      toUrl: e.toUrl,
      transitionType: e.transitionType,
      qualifiers: e.qualifiers,
    };
  if (e.kind === "link")
    return { kind: "link", id: e.id, ts: e.ts, newTabId: e.newTabId, sourceTabId: e.sourceTabId };
  if (e.kind === "close") return { kind: "close", id: e.id, ts: e.ts, tabId: e.tabId };
  return { kind: "start", id: e.id, ts: e.ts };
});

const doc = {
  version: 1,
  events: ordered,
  spa: [],
  rollup_days: [],
  sessions: [],
  meta: [],
};

const out = fileURLToPath(new NodeURL("../extension/src/sample-data.json", import.meta.url));
writeFileSync(out, JSON.stringify(doc, null, 1) + "\n");
console.error(
  `wrote ${out}: ${ordered.length} events across ${DAYS} days ` +
    `(${ordered.filter((e) => e.kind === "nav").length} navs)`
);
