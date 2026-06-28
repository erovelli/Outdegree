<!-- Thanks for contributing to Outdegree! Keep PRs focused and small. -->

## Summary

<!-- What does this change and why? Link any related issue (e.g. Closes #123). -->

## Type of change

- [ ] Bug fix
- [ ] New feature
- [ ] Refactor / internal
- [ ] Docs / tooling

## Checklist

I ran the same checks CI runs, and they pass:

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo test -p browsing-graph-core`
- [ ] `cargo clippy -p browsing-graph-core --lib --tests -- -D warnings`
- [ ] `cargo clippy -p browsing-graph-core --target wasm32-unknown-unknown --all-targets -- -D warnings`
- [ ] `npm run typecheck`
- [ ] `npm run test:ts`
- [ ] `npm run build` (and the manifest-privacy + network-surface audits still pass)

## Privacy invariants

- [ ] This change adds **no** network egress, telemetry, or analytics.
- [ ] `permissions`, `host_permissions`, CSP, and `incognito` are unchanged (or
      the change keeps them within the [invariants](../AGENTS.md#non-negotiable-invariants)).
- [ ] If I touched `derive.rs`/`rollup.rs`, the fold == recompute property test
      still passes.
- [ ] If user-visible, I updated `CHANGELOG.md`.
