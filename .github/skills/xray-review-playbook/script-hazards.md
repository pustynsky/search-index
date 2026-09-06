# Script Review Hazards

Read only the sections relevant to the diff. Treat these as failure hypotheses,
not proof that the current implementation has a defect.

## Safety Boundaries

- Validate only in disposable repositories and install directories under the
  resolved temporary root. Check both paths before executing a mutating test.
- Never run the installer on a real working tree, kill live Xray processes, install
  tooling, use elevation, change global Git configuration, or download a binary.
- Manual installer repros need `-SkipDownload`, explicit temporary `-RepoPath` and
  `-InstallDir`, and no `-KillRunning`. Inspect suites for equivalent isolation.
- Cleanup must validate containment of its target and avoid following junctions
  or symlinks into a real repository or installation.

## PowerShell

- Support Windows PowerShell 5.1 and PowerShell 7 where the touched behavior differs.
  Check `$PSNativeCommandUseErrorActionPreference`, `$ErrorActionPreference`, and
  `$LASTEXITCODE` together. Stderr redirection does not guarantee a failed native
  probe is harmless. Do not assume the same preference defaults on every host.
- Distinguish null from an empty collection; `@($null).Count` is one. Inspect
  empty JSON-container checks and object/array shapes after serialization.
- Parenthesize expressions such as `-not ($values -contains 'x')`.
- Check trailing newlines and encoding. Windows PowerShell 5.1 UTF-8 output can
  include a BOM; file consumers and byte-preservation contracts decide what is valid.
- Re-serializing upstream-managed JSON can change layout or shape. Tracked config
  must preserve canonical bytes through the filter contract; untracked JSON edits
  must preserve other servers and relevant array shapes.
- Check variables referenced across disabled blocks and early returns for missing
  initialization. Normalize paths before comparing Git and PowerShell output.
- Installation should be idempotent. Inspect duplicate filter sections, exclude or
  attribute entries, backups, partial installs, and explicit rollback behavior.
- For `$MyInvocation`, `$PSScriptRoot`, `$PSCommandPath`, or `$args` changes, inspect
  every relevant use under both file-backed and in-memory invocation.

## Filters And Embedded Perl

- Preserve `clean(smudge(canonical)) == canonical` byte-for-byte, filter idempotency,
  and embedded/canonical script consistency. Run applicable regression suites.
- Check `set -eu`, variable initialization, and `exec`: commands after `exec` do not
  run. A pipefail change needs evidence for the actual command pipeline.
- Verify snapshot lookup in normal repos and linked worktrees. Shared metadata
  belongs under the Git common directory where that is the installer contract.
- Preserve the documented optional-filter failure behavior (`filter.required=false`)
  and passthrough fallback. Verify both output bytes and Git's observed exit behavior.
- Filters need raw stdin/stdout handling for CRLF preservation. Replacing raw Perl
  processing with text tools needs byte-exact evidence on supported platforms.
- Check `*.sh text eol=lf`, shell quoting, Perl syntax, and strict/warnings usage.
- Use absolute end anchors such as `\z` when trimming absolute-end content. Check
  quoted braces and nested JSON instead of assuming regex brace counting is valid.
- Line-separator changes need LF and CRLF fixtures, plus relevant no-newline/BOM cases.

## Public Modes And Postconditions

For installer changes, derive affected invocation forms from current README/help,
not the requester's test selection. Mark each as tested, argued, or unverified:

- In-memory scriptblock or `iex` execution: a staged file run is not equivalent;
  command-path variables may be null.
- Downloaded standalone file without sibling filters: verify embedded fallback.
- Clone-mode script with sibling filters: verify canonical filter selection.
- Install from and run inside a linked worktree: verify common-dir resolution.

For each affected mode, consider clean/prior/legacy/partial installs, tracked and
untracked configs, both config targets, Visible/Hidden transitions, submodules,
restore, uninstall, and dry-run only where the changed invariant reaches them.
Check the promised result: Git cleanliness, canonical content, exclude/attribute
state, preserved unrelated servers, and successful rollback, not just exit zero.

## Security And Repository Gates

- Verify download source, asset selection, and integrity where changed. Do not
  execute fetched content during review or silently install after integrity failure.
- Treat repo/install paths and extension arguments as untrusted command inputs.
  Check PS 5.1 quoting as well as injection into stored Git filter commands/config.
- Recursive deletion must prove containment before acting and handle reparse points.
- User-facing changes require CHANGELOG coverage and the product-name gate;
  inspect changed files outside that gate's enumerated paths separately.
  New Markdown needs justification; preserve shell attributes and embedded-copy sync.