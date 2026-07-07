---
title: Privacy Policy
---

# Outdegree — Privacy Policy

_Last updated: 2026-07-07_

Outdegree records the web pages you navigate to and stores them **only on
your own device**, in your browser's local IndexedDB storage.

The extension makes **no network requests**. Your browsing data is never
transmitted, uploaded, collected, sold, or shared — it never leaves your
computer.

You can export your data to a local file, or delete it (in whole or by site) at
any time from the extension. Uninstalling the extension removes all stored data.

The extension does not run in Incognito mode and records nothing there.

## How this is enforced (not just promised)

- The extension declares **no host permissions** (`host_permissions: []`), so it
  has no granted ability to contact any website or server.
- Its Content Security Policy sets **`connect-src 'none'`**, so `fetch`,
  `XMLHttpRequest`, `WebSocket`, `EventSource`, and `navigator.sendBeacon` are
  **blocked by the browser** — even a misbehaving dependency cannot exfiltrate
  data.
- `"incognito": "not_allowed"` means incognito browsing is never observed.
- There are no content scripts, no `<all_urls>` access, and no remotely-hosted
  code. The Public Suffix List used for domain grouping is embedded at build
  time and never fetched.
- Site icons are displayed from **Chrome's own local favicon cache** (the
  `favicon` permission), served from the extension's own address — this makes **no
  network request** and fetches nothing from the internet. In browsers without
  that local service (e.g. Firefox) the icons are simply not shown.
- The only way data leaves the extension is a **user-initiated export**, which
  writes a file to your own disk — never a network upload.

## What is processed

Navigation metadata only: the URL you navigated to, the navigation type (e.g.
link, typed, form submit), and timing. To distinguish time actively spent on a
page from time it sat in a background tab, the extension also records which tab
and window are focused — as numeric browser ids and timestamps only, carrying no
URL or title. The extension does **not** read page content.

## Contact

Source code and issues: https://github.com/erovelli/Outdegree
