# Security Policy

Outdegree is a **local-only** browser extension: it records your navigations and
stores them **only on your own device**, and it makes **no network requests**.
Because it handles browsing data, its security and privacy posture is the whole
point — and it is **enforced by the browser and verified in CI**, not merely
promised.

## Reporting a vulnerability

Please report security issues **privately** via GitHub's
[private vulnerability reporting](https://github.com/erovelli/Outdegree/security/advisories/new)
(Security → Report a vulnerability). If that is unavailable, open a regular issue
that says only "security report — please enable advisories" without details, and
the maintainer will follow up.

- Please **do not** disclose publicly until a fix is available.
- Please **do not** attach exported browsing data or any personal data to a
  report — describe the issue and reproduction steps instead.
- This is an unpaid portfolio project: there is **no bug bounty**, but reports are
  genuinely appreciated and will be credited (with your consent).

Expect a best-effort acknowledgement within ~7 days.

## Supported versions

Only the latest released version receives fixes. There is no LTS branch.

## Threat model

**Assets:** your locally-stored navigation history (the `events`/`spa` stores and
derived rollups in IndexedDB).

**Trust boundary:** the browser's extension sandbox plus the extension's Content
Security Policy. The extension trusts the browser and the host OS.

**Guarantees (browser-enforced):**

| Guarantee | Mechanism |
|---|---|
| No network egress | `host_permissions: []` **and** CSP `connect-src 'none'` — `fetch`/`XHR`/`WebSocket`/`EventSource`/`sendBeacon` are blocked by the browser |
| Cannot read page content | no content scripts, no `<all_urls>`, no `web_accessible_resources`; capture uses `webNavigation` metadata only (URL + navigation type + timing) |
| No remote code | no remotely-hosted scripts; the Public Suffix List is embedded at compile time; WASM is instantiated from inlined bytes, not fetched |
| Never observes incognito | `incognito: "not_allowed"` |
| Only data-out path | a **user-initiated local file download** (export) — a `Blob` to disk, never an upload |

**Out of scope:** an attacker with physical/local access to an unlocked device; a
compromised browser, OS, or browser profile; malicious extensions with broader
permissions; and the contents of files the user themselves chooses to export and
share. These are outside what a sandboxed, local-only extension can defend
against.

## How the guarantees are verified

Every CI run (`.github/workflows/ci.yml`) gates the build on two bespoke audits:

1. **Manifest privacy audit** — runs against the *emitted* `dist/manifest.json`
   (not just source), failing the build if `host_permissions` is non-empty, if
   permissions stray outside `{ webNavigation, storage, unlimitedStorage }`, if
   the CSP loses `connect-src 'none'`, if `incognito` isn't `not_allowed`, or if
   content scripts / web-accessible resources appear.
2. **Network-surface audit** — greps the built `dist/` bundle for
   `fetch`/`XMLHttpRequest`/`WebSocket`/`EventSource`/`sendBeacon`/`importScripts`
   and for any external `http(s)://` URL, failing the build if any are present.

A contribution that weakens any of these will fail CI and will not be merged.
See [AGENTS.md](AGENTS.md#non-negotiable-invariants) for the full invariant list.
