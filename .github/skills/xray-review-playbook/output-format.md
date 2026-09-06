# Review Output

Lead with actionable findings and material gaps. Omit empty severity sections,
repeated checklists, and long narratives of correct code. "No findings" is valid.
Use concise technical language, with workspace-relative file links and line anchors.

## Severity And Verdict

| Severity | Evidence required |
| --- | --- |
| BLOCKER | Concrete security, soundness, corruption, data-loss, or destructive Git-state failure |
| MAJOR | Concrete correctness/contract regression, or a changed behavior lacking a test for an identified failure scenario |
| MINOR | Safe but consequential inefficiency or weak error behavior |
| NIT | Optional style/naming feedback; at most three, omitted when higher-severity defects exist |

- SHIP: no actionable defects, requested coverage complete, required gates satisfied.
- SHIP-WITH-NITS: only non-blocking observations remain and required gates are satisfied.
- BLOCK: a BLOCKER/MAJOR exists, or required validation/coverage is incomplete.
  Distinguish `code defect` from `validation/coverage incomplete` in the reason.
- Confidence reflects evidence quality, not severity. Use HIGH for complete,
  revision-matched inspection and validation; MEDIUM for bounded non-critical gaps;
  LOW for stale/partial evidence or unresolved material runtime behavior.
- Coverage: FULL for the requested changed surface and relevant direct consumers,
  PARTIAL for sampled/incomplete scope, DIFF-ONLY when surrounding code was not read.

## Finding Contract

For each BLOCKER/MAJOR, name the location, concrete failing scenario, affected
contract, supporting evidence, why current tests do not prevent it, and the specific
change needed. Keep one concern per finding. An inference must be labeled as such.
For optional feedback, use `nit: Suggestion:` without changing its original severity.

## Compact Report

```text
Findings: <severity, file link, failure mode, evidence, suggested change>
Coverage gaps: <only material unknowns, unavailable checks, or out-of-scope surfaces>
Verdict: SHIP | SHIP-WITH-NITS | BLOCK
Reason: <1-2 sentences; code defect or validation/coverage incomplete if blocked>
Confidence: HIGH | MEDIUM | LOW
Coverage: FULL | PARTIAL | DIFF-ONLY; <revision, diff source, reviewed scope>
Validation: <command, provenance, exit code, concise observed result>
```

Use `RUN BY REVIEWER` or `SUPPLIED, HASH-MATCHED` per check. If no commands were
executed by the reviewer, state `Build/Test evidence: NOT RUN BY REVIEWER` and list
any supplied evidence separately. State skips and whether each blocks the verdict.
For installer changes, include a compact affected-entry-point matrix with rows
marked tested, argued, or unverified and the corresponding test/repro or gap.

For FOLLOW-UP, add one short prior-finding resolution table; do not repeat unchanged
findings or previously cleared files. Keep negative evidence to at most five points.
For a clean small review, aim for a few paragraphs. Expand only for actionable
findings or evidence the requester needs to evaluate the verdict.