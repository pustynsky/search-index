# Rust Review Hazards

Use the applicable rows, not an exhaustive checklist for every diff. Resolve any
project-profile fact against current Cargo metadata and code before citing it.

## Contracts And Runtime

- Xray is a single crate with library and binary targets and a synchronous runtime.
  Do not assume there is an async runtime or that production code contains no unsafe.
- Check `edition`, `rust-version`, enabled `lang-*` features, and dependency versions
  in Cargo metadata when affected. Raising the MSRV needs an intentional change and
  release documentation; do not hardcode a remembered version in the review.
- Preserve JSON-RPC envelopes, MCP schemas, CLI flags, stdout/stderr roles, and exit
  codes. Check compatibility of changed fields and defaults with direct consumers.
- Positional bincode layouts are order-sensitive. Field additions, removals, or
  reordering need format-version and invalidation/migration handling. A serde
  default alone does not make a positional format backward-compatible.
- For persisted changes, verify old/missing/corrupt/stale state and reload behavior.
  Distinguish rebuildable cache invalidation from migration of non-rebuildable data.

## Correctness And Safety

- Check ownership, lifetime assumptions, error propagation, and recoverable paths
  that now panic. Inspect full relevant bodies rather than isolated diff lines.
- For locks and atomics, check ordering, poisoning, nested locks, and shutdown.
  Watcher changes need relevant delete-during-read, write-during-parse, save-storm,
  restart, and stale-cache scenarios.
- Windows paths need consistent normalization, case handling, UNC and drive-relative
  semantics. Test fixture roots should be canonical where the production contract is.
- For model-supplied writes/deletes, verify the actual workspace authorization
  contract, traversal handling, and symlink/junction escapes. Canonicalization alone
  does not resolve a time-of-check/time-of-use race.
- Check new external commands for argument injection. Pass arguments separately
  rather than interpolating untrusted values into a shell command.
- For supplied regexes and large inputs, check pattern-size, compiled-size, memory,
  and output bounds. Establish the engine's complexity behavior before claiming ReDoS.
- Check response/log changes for unintended secrets, environment values, and paths.
- Production unsafe includes Win32 calls and allocator interaction. New or changed
  unsafe needs a concise `SAFETY` explanation and valid layouts, handles, lifetimes,
  alignment, and buffer bounds, including error paths.

## Performance And Dependencies

- On hot paths, inspect added clones, allocations, scans, unbounded growth, and
  algorithmic complexity. Report a concrete workload, not speculative micro-optimizations.
- Parser or tree-sitter changes require grammar/core compatibility evidence.
- Language-feature changes must preserve no-default and individual-feature builds;
  new language support also needs dependency, registration, and gated tests.
- Build-script changes must remain deterministic, with correct rerun directives;
  avoid newly required network, timestamps, or machine-specific state.
- Allocator changes should not create assumptions about address order or zero-init.
- Dependency and lint-policy changes need relevant advisory/license evidence and
  consistency with `deny.toml` and `clippy.toml`.

## Tests And Repository Rules

- Tests should exercise the changed branch and observable result. Check that a
  fallback or inverted condition would not make a regression test pass accidentally.
- Windows drive-letter or UNC literals need Windows gating or portable construction.
- User-facing changes require CHANGELOG coverage and the product-name gate;
  inspect changed files outside that gate's enumerated paths separately.
- New Markdown files need justification; do not bypass repository hooks.
- Never run or recommend `cargo fmt` or `rustfmt`. Unrelated formatting churn is
  review noise to identify, not a reason to rewrite surrounding files.