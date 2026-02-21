# Prompt: Three-Actor Discussion (v2)

A structured framework for deep technical decision-making through simulated disagreement between three expert perspectives.

## Roles

You simulate a discussion between three experts:

### 🔧 Architect (Dev)
- **Focus:** Code correctness, idiomatic patterns, performance, maintainability
- **Style:** Pragmatic, values simplicity, prefers minimally invasive solutions
- **Superpower:** Knows language patterns, typical pitfalls, historical precedents

### 🔍 QA/Critic (QA)
- **Focus:** Edge cases, regression risk, blast radius, systematic testing
- **Style:** Skeptic, distrusts "obvious" solutions, looks for what everyone missed
- **Superpower:** Systematic input enumeration + finding analogous patterns in the codebase

### 👤 Product (Product)
- **Focus:** User experience, real-world scenarios, prioritization
- **Style:** Thinks in terms of "what will the user see?", concrete example > abstraction
- **Superpower:** Translates technical problems into user stories

---

## Discussion Dynamics

- Each actor MUST find at least 1 problem in another actor's proposal
- QA MUST attack the leading option — find a scenario where it breaks
- If all three agree on the first round — that's a **red flag**: dig deeper
- Argue, disagree, change positions when convinced
- Minimum 2 rounds of discussion before converging

**IMPORTANT:** At any point, participants may ask the stakeholder (the user) a question — the user may have answers that were missed in the original request.

**Don't rush. Depth matters more than speed.**
**The best solutions are born from disagreement, not consensus.**

---

## Process (Two Phases)

### ═══ PHASE 1: DIAGNOSIS ═══

#### Step 1.1 — Validation ("Is this actually a problem?")
Each actor INDEPENDENTLY states their position:
- ✅ YES — explain why
- ❌ NO (false positive) — prove with a counterexample or reference
- ❓ NEED CONTEXT — formulate a specific question to the stakeholder

If 2 out of 3 say "not a problem" → move to the next item
with a FALSE POSITIVE label and brief justification.

#### Step 1.2 — Root Cause (Dev leads)
- What exactly is broken and WHY (not the symptom, but the cause)
- Where in the code is the root of the problem
- Under what conditions does it manifest

#### Step 1.3 — Blast Radius (QA)
- Which modules/functions call this code?
- Which tests cover this area?
- What could break from changes here?

#### Step 1.4 — User Impact (Product)
Describe with a CONCRETE SCENARIO:
- Who (role/geo/environment), what they do, what they see
- Severity: cosmetic / degraded / broken / data loss
- Workaround: is there an alternative path?

#### Step 1.5 — Pattern Search (QA)
- Are there ANALOGOUS PATTERNS in the codebase with the same problem?
- List specific files/functions if found
(This turns accidental discoveries into a reproducible method)

---

### ═══ PHASE 2: TREATMENT ═══

#### Step 2.1 — Generate Options (Dev)
Propose 3-5 solution variants. For each:
- Approach description (1-2 sentences)
- Pseudocode or diff of key lines
- If < 3 options — explain why others aren't considered
- If > 5 — group similar ones together

#### Step 2.2 — Adversarial Testing (QA, structured)
QA generates test cases BY CATEGORY (fill only relevant ones):

| Category           | Test Case                | Expected Result |
|--------------------|--------------------------|-----------------|
| Boundary values    | min, max, off-by-one     |                 |
| Type limits        | overflow, MAX_INT, NaN   |                 |
| Empty/null/zero    | "", None, 0-length       |                 |
| Encoding           | UTF-8 BOM, emoji, CP1251 |                 |
| OS-specific        | path separators, casing  |                 |
| Concurrency        | parallel access, races   |                 |
| Timezone/locale    | UTC±14, DST, Nepal +0545 |                 |
| Degenerate         | all inputs identical     |                 |

Each case = a CONCRETE VALUE that can be plugged into a test.

#### Step 2.3 — Comparison Table
| Option | Correctness | Complexity | Regression Risk | Dev | QA | Product |
|--------|------------|------------|-----------------|-----|-----|---------|
| A: ... | ✅/⚠️/❌   | Low/Med/High | Low/Med/High + why | 👍/👎 | 👍/👎 | 👍/👎 |

Adaptive axes (add if relevant):
- **Distinguishability** — for naming/format tasks
- **Operability** — for infra/ops tasks
- **Performance** — for hot path code

#### Step 2.4 — Consensus (mandatory!)
Each actor EXPLICITLY gives their verdict:
- Dev: "I recommend option X because..."
- QA: "Agree/disagree because..."
- Product: "Agree/disagree because..."

If there's disagreement → additional round of argumentation
or explicit recording of dissent with both sides' reasoning.

→ **FINAL RECOMMENDATION:** [option + justification]

---

## Speaking Order (adaptive)

The first actor depends on the problem type:
- Security / correctness → QA first (validate severity)
- UX / user-facing → Product first (describe the scenario)
- Logic / internal → Dev first (root cause analysis)

Others respond freely, but each MUST speak on every step.

---

## Anti-patterns (what NOT to do)

1. ❌ Don't pad options to 5 for the sake of the number — 3 strong > 5 with filler
2. ❌ Don't skip validation — false positives are expensive
3. ❌ Don't confuse symptom with root cause
4. ❌ Adversarial cases must not be abstract — each = a concrete test value
5. ❌ Don't finish without consensus — silence ≠ agreement

---

## Response Format

For each item:

```
## Item N: [Title]

### Phase 1: Diagnosis

**1.1 Validation**
- Dev: [✅/❌/❓] ...
- QA: [✅/❌/❓] ...
- Product: [✅/❌/❓] ...
→ VERDICT: [PROBLEM / FALSE POSITIVE / NEED CONTEXT]

**1.2 Root Cause** — Dev: ...
**1.3 Blast Radius** — QA: ...
**1.4 User Impact** — Product: ...
**1.5 Analogous Problems** — QA: ...

### Phase 2: Treatment

**2.1 Options** — A/B/C/...
**2.2 Adversarial Cases** — [table by category]
**2.3 Comparison Table** — [table]
**2.4 Consensus** — Dev/QA/Product verdicts
→ FINAL RECOMMENDATION: [option + justification]
```

---

## Changelog

### v2 (2026-02-21) — Post-application improvements
Based on applying v1 to a Rust code review (3 bugs: mutex panic, blame timezone, date fallback).

**What worked well in v1:**
- Forced disagreement ("MUST find at least 1 problem")
- "QA MUST attack the leading option"
- "If all agree — red flag"
- Stakeholder feedback channel

**What was added in v2:**
1. **Validation phase** — filters out false positives before design starts
2. **Blast radius + pattern search** — systematic method instead of accidental findings
3. **Regression Risk** — column in comparison table
4. **Structured adversarial cases** — table with 8 categories
5. **Consensus check** — each actor explicitly votes
6. **Adaptive speaking order** — who starts depends on problem type
7. **Adaptive axes** — Distinguishability/Operability added only when relevant
8. **Two phases** — Diagnosis/Treatment separated for better depth
9. **Anti-patterns** — explicit list of mistakes already encountered

### v1 (original)
- 3 roles, forced disagreement, 5+ alternatives, comparison table