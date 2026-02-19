# Validation Protocol for search-index: search_callers & search_definitions

## Purpose

Systematic detection of false positives and false negatives in `search_callers` (direction=up/down) and `search_definitions` tools. This protocol is designed for **LLM-assisted** execution: the LLM reads source code, builds ground truth, runs search-index, and compares results.

## When to Execute

- After any change to parsers (`parser_typescript.rs`, `parser_csharp.rs`)
- After changes to `callers.rs` (resolver, caller/callee trees)
- After changes to `watcher.rs` (incremental index updates)
- Before releasing a new version of search-index

## Language Relevance Map

> **For LLM**: If the user specified a target language, use this table to skip irrelevant sections.

| Section | Universal | TypeScript | C# |
|---------|:---------:|:----------:|:--:|
| **Part 1**: Criteria 1-6 (popular names, local vars, inheritance, DI, multi-class) | ✅ | ✅ | ✅ |
| **Part 1**: Criterion 7 (Angular inject) | — | ✅ | — |
| **Part 1**: Criteria 8-10 (var inference, extension methods, partial classes) | — | — | ✅ |
| **Part 2**: Validation procedure (Phases 1-4) | ✅ | ✅ | ✅ |
| **Part 3**: Report format | ✅ | ✅ | ✅ |
| **Part 4**: Limitations 1-3 (method chains, generics, dynamic dispatch) | ✅ | ✅ | ✅ |
| **Part 4**: Limitations 4-5 (destructuring, higher-order functions) | — | ✅ | — |
| **Part 4**: Limitation 9 (module imports) | — | ✅ | — |
| **Part 4**: Limitations 10-12 (using static, extension methods, LINQ) | — | — | ✅ |
| **Part 4b**: Anti-patterns | ✅ | ✅ | ✅ |
| **Part 5**: Execution template | ✅ | ✅ | ✅ |
| **Part 6**: Example execution (template + discovery) | ✅ | ✅ | ✅ |
| **Part 7**: Baseline tests (discovery-based) | ✅ | ✅ | ✅ |
| **Part 8**: CI/CD integration | ✅ | ✅ | ✅ |

---

## Part 1: File Selection for Testing

### Selection Criteria

Choose files that **maximize the probability of bug detection**:

| #   | Criterion                            | Why it matters                               | What it tests                                                              |
| --- | ------------------------------------ | -------------------------------------------- | -------------------------------------------------------------------------- |
| 1   | **Popular method names**             | Highest risk of false matches                | `resolve`, `get`, `set`, `execute`, `init`, `update`, `validate`, `handle` |
| 2   | **Calls via local variables**        | Tests receiver_type resolution               | `const x = new Foo(); x.bar()`                                             |
| 3   | **Large class** (300+ lines)         | More call sites = more chances for errors    | Components with ngOnInit, services                                         |
| 4   | **Inheritance**                      | Tests base_types resolution                  | `class Sub extends Base { this.method() }`                                 |
| 5   | **Constructor injection (DI)**       | Tests DI-awareness                           | `constructor(private svc: ISomeService)`                                   |
| 6   | **Multiple classes in one file**     | Tests file-level vs class-level filtering    | Helper classes, enums in the same file                                     |
| 7   | **Angular inject() / @Inject** (TS)  | Tests inject patterns                        | `private svc = inject(SomeService)`                                        |
| 8   | **var type inference** (C#)          | Tests `var x = new Foo()` extraction         | `var result = service.GetUser(); result.Validate()`                        |
| 9   | **Extension methods** (C#)           | Call without visible receiver                | `list.Where(x => ...).Select(...)` — LINQ                                 |
| 10  | **Partial classes** (C#)             | Definition split across files                | `partial class Foo` in two .cs files                                       |
| 11  | **Bare function calls**              | Calls without receiver                       | `someFunction()` instead of `this.someFunction()`                          |

### File Profile Categories

To ensure statistical significance, each validation run MUST test files from **at least 5 of the 6 profile categories** below. Single-file validation is insufficient — different code patterns exercise different parser paths.

| Profile | Description | What it stresses | Example search |
| ------- | ----------- | ---------------- | -------------- |
| **P1: Large class** | Class with >20 methods | Many call sites, intra-class calls | `search_definitions kind=class` → pick where `line_end - line_start > 300` |
| **P2: DI dependencies** | Class with `[ServiceDependency]` fields (C#) or constructor injection (TS) | DI-aware receiver resolution | `search_grep terms='ServiceDependency' showLines=true maxResults=10` |
| **P3: Method overloads** | Class with multiple methods sharing the same name but different signatures | Overload disambiguation | `search_definitions parent=<Class> kind=method` → look for duplicate names |
| **P4: Factory / local vars** | Class using `var x = factory.Create()` or `const x = new Foo()` patterns | Local variable type inference | `search_grep terms='= new,factory.Create,Builder.Build' showLines=true` |
| **P5: Interface + implementations** | Interface with ≥2 implementing classes | DI interface resolution, cross-class caller matching | `search_definitions kind=interface` → `search_definitions baseType=<IInterface>` |
| **P6: Deep call chains** | Class where method A → B → C → D (>3 levels deep) | Multi-depth caller tree accuracy | `search_callers method=<leaf> class=<Class> direction=up depth=4` |

### Recommended Sample Size

- **Minimal check**: 5 files covering ≥5 profiles (quick smoke validation, ~45 min)
- **Standard check**: 8-10 files covering all 6 profiles (main validation, ~2 hours)
- **Full check**: 15-20 files, ≥2 files per profile (before release, ~4 hours)

> **Rule**: A validation report that tests fewer than 5 files or covers fewer than 5 profiles is considered **incomplete** and MUST NOT be used for release sign-off.

### How to Choose Files

**Step 1**: Find files with popular method names

```
search_definitions name='resolve,execute,validate,handle,get,update' kind=method maxResults=20
```

**Step 2**: From results, select files with different characteristics (DI, inheritance, size). Tag each selected file with its profile(s) from the table above.

**Step 3**: Add files from any uncovered profiles until ≥5 profiles are represented.

**Step 4**: Record the profile coverage matrix in the report header (see Part 3).

---

## Part 2: Validation Procedure

### Phase 1: Ground Truth Extraction

Goal: LLM reads source code and builds a "reference" — a list of all method calls with their receivers.

**Step 1.1**: Get the file structure

```
search_definitions file='TargetFile.ts' includeBody=true maxBodyLines=50
```

**Step 1.2**: For each method in the file, the LLM analyzes the body and creates a table:

| Container method | Line | Call               | Receiver expression    | Expected receiver type | Category      |
| ---------------- | ---- | ------------------ | ---------------------- | ---------------------- | ------------- |
| `ngOnInit`       | 376  | `createActivity()` | `this.activityService` | `ActivityService`      | DI field      |
| `ngOnInit`       | 377  | `resolve()`        | `activity`             | `Activity`             | local var     |
| `ngOnInit`       | 380  | `subscribe()`      | `this.store.select(…)` | `Observable`           | method chain  |
| `getData`        | 120  | `parseInt()`       | —                      | —                      | bare function |

**Call categories**:

- `this.field` — class field (DI or regular)
- `this.method()` — call to own method
- `local var` — call on a local variable
- `static` — static call (`ClassName.method()`)
- `bare function` — call without receiver
- `method chain` — chained calls (`a.b().c()`)
- `super` — parent method call

**Step 1.3**: Record ground truth in structured form

### Phase 2: Direction=Down Verification

Goal: for each method in the file, verify that `search_callers direction=down` returns correct callees.

**Step 2.1**: Run the search

```
search_callers method=<method_name> class=<class_name> direction=down depth=1
```

**Step 2.2**: For each returned callee, verify:

| Check                           | How                                          | Result                         |
| ------------------------------- | -------------------------------------------- | ------------------------------ |
| Callee exists in ground truth?  | Compare method_name + callSiteLine           | TRUE POSITIVE / FALSE POSITIVE |
| callSiteLine correct?           | Compare with line in ground truth            | CORRECT / WRONG LINE           |
| Receiver type correct?          | Compare callee parent with expected receiver | CORRECT / WRONG RECEIVER       |

**Step 2.3**: Check for missed calls (false negatives)

For each call in ground truth that is NOT in `search_callers` results:

- Record as FALSE NEGATIVE
- Classify the cause: `local var`, `method chain`, `generic return type`, `bare function`, other

### Phase 3: Direction=Up Verification

Goal: for selected methods, verify that `search_callers direction=up` returns only real callers.

**Step 3.1**: Select 3-5 methods for verification. Priority:

- Methods with popular names (highest false positive risk)
- Methods called via inheritance

**Step 3.2**: Run the search

```
search_callers method=<method_name> class=<class_name> direction=up depth=1
```

**Step 3.3**: For each returned caller, **read the caller's source code**:

```
search_definitions name=<caller_method> parent=<caller_class> includeBody=true
```

**Step 3.4**: Verify:

| Check                                | How to verify                                                          |
| ------------------------------------ | ---------------------------------------------------------------------- |
| Call exists?                         | Caller body contains a line with `<method_name>(`                      |
| Call targets the correct class?      | Receiver = this / field of type <class_name> / localVar of type <class_name> |
| Call is not in a comment?            | Line does not start with `//` or `/*`                                  |
| Receiver is not a different class?   | `path.resolve()` ≠ `MyTask.resolve()`                                  |

Result: TRUE POSITIVE or FALSE POSITIVE (with reason)

### Phase 4: search_definitions Verification

Goal: ensure all definitions in the file are correctly indexed.

**Step 4.1**: LLM reads the file and lists all definitions:

- Classes, interfaces, enums
- Methods, properties, constructors
- Base types (extends/implements)

**Step 4.2**: Compare with results:

```
search_definitions file='TargetFile.ts' maxResults=200
```

**Step 4.3**: Verify:

- All classes found?
- All methods found?
- Base types correct?
- line_start / line_end correct?
- parent correctly assigned?

---

## Part 3: Report Format

### File Path Convention

> **Rule**: ALL file references in validation reports MUST use **full absolute Windows paths** (e.g., `C:\Repos\MyProject\src\Services\OrderService.cs`). Relative paths or short filenames (e.g., `OrderService.cs`) are **not acceptable** — they are ambiguous in a monorepo with duplicate filenames across modules.

This applies to:
- File paths in per-file summary tables
- File paths in error detail sections
- File paths returned by `search_definitions` / `search_callers` (copy the `file` field as-is)
- Ground truth tables

### Header

```
# search-index Validation — [Date]
## Indexed project: [name, path]
## search-index version: [commit hash or version]
## Profile coverage: P1 ✅ P2 ✅ P3 ✅ P4 ✅ P5 ✅ P6 ❌
## Files tested: [N] (minimum 5)
```

### Per-file Summary

```markdown
### File: `C:\Repos\MyProject\src\Module\Services\MyService.cs`
### Profiles: P1 (large class), P2 (DI dependencies)

| Class   | Methods | Ground Truth calls | Down TP | Down FP | Down FN | Up TP | Up FP |
| ------- | ------- | ------------------ | ------- | ------- | ------- | ----- | ----- |
| MyClass | 12      | 45                 | 38      | 2       | 5       | 8     | 1     |
```

> Note: Always use the full absolute path, never `MyService.cs` alone.

### Error Details

```markdown
#### FALSE POSITIVE (Down): `MyClass.ngOnInit` → `FileProcessor.resolve`

- **callSiteLine**: 377
- **Cause**: `activity.resolve()` — local variable, receiver_type=None → accepted all candidates
- **Pattern**: LOCAL_VAR_NO_TYPE
- **Severity**: Medium

#### FALSE NEGATIVE (Down): `MyClass.ngOnInit` → `Observable.subscribe`

- **Line**: 380
- **Cause**: Method chain — `this.store.select(...).subscribe(...)` — chain not tracked
- **Pattern**: METHOD_CHAIN
- **Severity**: Low (expected limitation)

#### FALSE POSITIVE (Up): `BuildServer.initTools` → `BaseTask.resolve`

- **callSiteLine**: 36
- **Cause**: `path.resolve()` in a file that imports BaseTask
- **Pattern**: WRONG_RECEIVER_SAME_FILE
- **Severity**: High
```

### Precision and Recall Formulas

All metrics MUST use exact integer counts, not approximations.

**Definitions:**

- **TP (True Positive)**: A result returned by the tool that is confirmed correct by ground truth
- **FP (False Positive)**: A result returned by the tool that does NOT exist in ground truth (wrong receiver, comment match, etc.)
- **FN (False Negative)**: A call/definition that exists in ground truth but was NOT returned by the tool

**Formulas:**

$$Precision = \frac{TP}{TP + FP}$$

> *What fraction of returned results are correct?* High precision = few false positives.

$$Recall = \frac{TP}{TP + FN}$$

> *What fraction of expected results were returned?* High recall = few false negatives.

**How to count per test:**

| Context | TP | FP | FN |
| ------- | --- | --- | --- |
| **Direction=Down** | Callee in ground truth AND in search results | Callee in search results but NOT in ground truth | Callee in ground truth but NOT in search results |
| **Direction=Up** | Caller confirmed to call the target method | Caller does NOT actually call the target method | (Not measured — requires exhaustive codebase annotation) |
| **Definitions** | Definition in source AND in index | Definition in index but NOT in source (phantom) | Definition in source but NOT in index (missed) |

> **Rule**: Report exact counts (e.g., `Precision = 38/40 = 95.0%`), not rounded approximations like `~95%`.

### Summary Metrics Table

```markdown
## Summary Metrics

| Metric          | Direction=Down       | Direction=Up       | Definitions        |
| --------------- | -------------------- | ------------------ | ------------------ |
| Total TP        | 38                   | 8                  | 195                |
| Total FP        | 2                    | 1                  | 0                  |
| Total FN        | 5                    | —                  | 3                  |
| Precision       | 95.0% (38/40)        | 88.9% (8/9)        | 100.0% (195/195)   |
| Recall          | 88.4% (38/43)        | —                  | 98.5% (195/198)    |
| Files tested    | 8                    | 8                  | 8                  |
| Profiles covered| P1,P2,P3,P4,P5      | P1,P2,P5           | P1-P6              |
```

### Action Items / Recommendations

Every validation report MUST include this section. It captures actionable findings that need follow-up.

```markdown
## Action Items

### Critical Bugs (must fix before release)

| # | Severity | Description | Affected Test(s) | Affected File(s) | Suggested Fix |
|---|----------|-------------|-------------------|-------------------|---------------|
| 1 | 🔴 Critical | `search_callers` returns phantom callers from deleted methods | B3 (regression), File P2 | `C:\Repos\...\MyService.cs` | Invalidate stale entries in watcher.rs on file delete |

### Known Limitations (documented, with workarounds)

| # | Severity | Description | Affected Test(s) | Workaround |
|---|----------|-------------|-------------------|------------|
| 1 | 🟡 Medium | Method chains (`a.b().c()`) not tracked | B1, B5 | Use `direction=down depth=1` and manually trace intermediate types |
| 2 | 🟡 Medium | Module imports (`path.resolve()`) produce FP when class inherits from target | B4 | Manually verify callers with popular method names |

### Improvement Opportunities (nice-to-have)

| # | Severity | Description | Affected Test(s) | Suggested Fix |
|---|----------|-------------|-------------------|---------------|
| 1 | 🟢 Low | Generic return type inference would eliminate ~10% of FN | B5 | Track generic type parameters in parser |
| 2 | 🟢 Low | LINQ query syntax not parsed as method calls | — | Add query expression visitor to C# parser |
```

> **Rule**: If no items exist in a category, write "None" — do not omit the category. Every report must show all three categories.

**Severity levels:**
- 🔴 **Critical**: Produces incorrect results that could mislead users; must fix before release
- 🟡 **Medium**: Known limitation with a workaround; document and track
- 🟢 **Low**: Enhancement opportunity; nice-to-have for future iterations

### Error Pattern Classification

```markdown
## Error Patterns (by frequency)

| #   | Pattern                  | Direction | Frequency | Description                                        | Fix Status                              |
| --- | ------------------------ | --------- | --------- | -------------------------------------------------- | --------------------------------------- |
| 1   | LOCAL_VAR_NO_TYPE        | Down FP   | ~~35%~~   | receiver_type=None for local variables             | ✅ Fix 1/1b (local var types)           |
| 2   | WRONG_RECEIVER_SAME_FILE | Up FP     | ~~25%~~   | path.resolve() in file with BaseTask               | ✅ Fix 3a (verify_call_site_target)     |
| 3   | COMMENT_MATCH            | Up FP     | ~~15%~~   | Token in comment, not in code                      | ✅ Fix 3a (AST verification)            |
| 4   | METHOD_CHAIN             | Down FN   | 15%       | a.b().c() — intermediate types not tracked         | Parser limitation                       |
| 5   | GENERIC_RETURN           | Down FN   | 10%       | createActivity<T>() — generic return type          | Parser limitation                       |
| 6   | CROSS_CLASS_POLLUTION    | Down FP   | ~~20%~~   | depth=2 pulls same-named methods from other classes | ✅ Fix 3c (caller_parent scoping)       |
| 7   | GENERIC_NAME_COLLISION   | Down FP   | ~5%       | `new List<T>()` → non-generic user-defined List    | ✅ Fix 3d (generic arity check)         |
| 8   | OVERLOAD_DEDUP           | Def FN    | ~~5%~~    | Method overloads collapsed to one entry (dedup removed duplicates) | ✅ Fixed: dedup keys now include line_start |
| 9   | SAME_NAME_DIFFERENT_RECEIVER | Up FP | ~~5%~~    | Callers through unrelated interface matched due to same method name | ✅ Fixed: interface resolution filtered by relatedness + parser preserves unresolved receiver names |
```

---

## Part 4: Known Parser Limitations

These cases **are not bugs** — they are beyond tree-sitter parser capabilities. Do NOT mark them as errors in the report.

| #   | Limitation                 | Example                                          | Why it can't be fixed              |
| --- | -------------------------- | ------------------------------------------------ | ---------------------------------- |
| 1   | **Method chains**          | `a.getB().doC()` — type of `getB()` unknown      | Needs type checker, not parser     |
| 2   | **Generic return types**   | `createActivity<T>()` → return value type        | Needs generic inference            |
| 3   | **Dynamic dispatch**       | `obj[methodName]()`                              | Method name computed at runtime    |
| 4   | **Spread/destructuring**   | `const { method } = service; method()`           | Receiver lost during destructuring |
| 5   | **Higher-order functions** | `const fn = this.method; fn()`                   | Receiver lost during assignment    |
| 6   | **Conditional types**      | `const x = cond ? new A() : new B(); x.method()` | Two possible types                 |
| 7   | **Type assertions**        | `(x as SomeType).method()`                       | Parser doesn't track type assertions |
| 8   | **BCL/framework types**    | `new List<T>()` when generic List<T> not in index but non-generic `List` exists | Generic arity check handles most cases; same-arity collisions remain |
| 9   | **Module imports** (TS)    | `import * as path from 'path'; path.resolve()` → receiver_type=None | Module imports not tracked; tree-sitter can't distinguish modules from local variables |
| 10  | **Static using** (C#)      | `using static System.Math; Abs(-1)` → receiver_type=None            | Static imports in C# are not tracked as receivers                                      |
| 11  | **Extension methods** (C#) | `list.Where(x => ...)` → receiver_type=List, but method is in Enumerable | Extension methods are called on one type but defined in another                  |
| 12  | **LINQ query syntax** (C#) | `from x in list where x > 0 select x` — no explicit method call     | Query syntax is translated to method calls by the compiler, not the parser             |
| 13  | **Local var receiver names** | `var x = GetService(); x.DoWork()` — type unknown but name preserved | As of v1.x, the parser preserves the receiver variable name (e.g., `receiver_type = Some("dbSession")`) even when the type cannot be inferred. This allows downstream verification to correctly reject mismatched receivers. However, calls through local variables still won't resolve to specific definitions in direction=down because the actual type is unknown. |

When an error is found, first check if it falls into this table. If yes, mark as "expected limitation", not a bug.

---

## Part 4b: Implementation Anti-patterns Discovered

Lessons learned during fix validation. Watch for these in future changes.

### Anti-pattern 1: "Too Generous Fallback"

**Discovered in**: `verify_call_site_target()`, line 354 of `callers.rs`

**Problem**: Code `if matching_calls.is_empty() { return true; }` accepts ALL cases where there is no call-site data on a specific line — including comments.

**Correct**: `return call_sites.is_empty()` — accept only if the method has no call-site data at all (graceful fallback), but reject if data exists but nothing matches on this line (= comment/non-code).

**Pattern**: When implementing fallback logic, distinguish:
- "No data at all" → fallback to acceptance (can't verify)
- "Data exists but not on this line" → reject (verified: not a real call)

### Anti-pattern 2: "Inheritance Masks Module Import"

**Discovered in**: `SubTask.resolvePath` → `path.resolve()`

**Problem**: `SubTask extends BaseTask`. When `path.resolve()` has `receiver_type=None` (module `path` is not tracked), the fallback logic checks "does caller inherit from target?" → yes → accepts.

**Why it's hard to fix**: Need to distinguish `this.resolve()` (legitimate inheritance call) from `path.resolve()` (call on a module object). Both have `receiver_type=None`. For proper filtering, need to:
1. Track module imports as types, OR
2. Check the AST: the call expression has a member_expression with object `path` → it's not a bare call

### Diagnostic Technique: callSite Line Shift

When re-running a test after changes, **watch for changes in callSiteLine**:
- `callSite: 397 → 446` means: line 397 was filtered out, but the same caller method was found via a different match (line 446)
- This is a useful indicator that filtering works, but there are additional entry paths

---

## Part 5: Execution Template

### Quick Checklist (3 files, ~30 min)

```
□ 1. Select 3 files using criteria from Part 1
□ 2. For each file:
  □ 2.1. search_definitions file=<file> includeBody=true → read
  □ 2.2. Build ground truth (call table)
  □ 2.3. search_callers direction=down depth=1 for the main method
  □ 2.4. Compare results → record FP/FN
  □ 2.5. search_callers direction=up depth=1 for 2 methods
  □ 2.6. Read each caller → record FP
□ 3. Fill in the summary metrics table
□ 4. Classify found errors by patterns
```

### Full Check (10+ files, ~2 hours)

```
□ 1. Select 10-15 files covering all criteria from Part 1
□ 2. For each file: full procedure from Part 2 (all 4 phases)
□ 3. Fill in the report per Part 3 format
□ 4. Compare metrics with previous run (if available)
□ 5. Record regressions
□ 6. Create issues for new bugs
```

---

## Part 6: Example Execution (discovery-based)

### How to Find a Candidate File

```
Step 1: Find a large class with DI
  search_definitions kind=class maxResults=10
  → select a class with 300+ lines (line_end - line_start > 300)

Step 2: Check its methods
  search_definitions parent=<SelectedClass> kind=method maxResults=20
  → select the method with the largest body (lifecycle hook or init-like method)

Step 3: Read the method body
  search_definitions name=<Method> parent=<Class> includeBody=true
  → build ground truth call table
```

### Ground Truth Template

| Line | Call | Receiver expression | Expected type | Category |
|------|------|---------------------|---------------|----------|
| N | `someMethod()` | `this.injectedService` | `InjectedServiceType` | DI field |
| N+1 | `process()` | `localVar` | `SomeType` | local var |
| N+3 | `getData()` | `this` | `<SelectedClass>` | this.method |
| N+5 | `subscribe()` | `this.store.select(...)` | `Observable` | method chain |

### Direction=Down Verification Template

```
search_callers method=<Method> class=<Class> direction=down depth=2
```

**Check for each callee**:
- ✅ Callee exists in ground truth? → TRUE POSITIVE
- ❌ Callee NOT in ground truth? → FALSE POSITIVE, record reason
- All calls from ground truth found? → if not, record FALSE NEGATIVE

**Typical FPs that should be eliminated by fixes**:
- `<localVar>.<method>()` → previously showed all definitions of `method()` from the index → now only from the correct class
- Same-named methods from unrelated classes → filtered by caller_parent scoping

### Direction=Up Verification Template

```
Step 1: Select a method with a "popular" name
  search_definitions name='resolve,execute,validate,handle,get' kind=method maxResults=20
  → select a method from a class that has subclasses (base_types)

Step 2: Run the search
  search_callers method=<Method> class=<Class> direction=up depth=1

Step 3: For each caller — read source code
  search_definitions name=<CallerMethod> parent=<CallerClass> includeBody=true

Step 4: Verify each caller
```

**Check for each caller**:
- ✅ Caller body contains a call to `<Class>.<Method>()` (via this, DI, inheritance) → TRUE POSITIVE
- ❌ Call on a different object (e.g., `path.resolve()` instead of `MyClass.resolve()`) → FALSE POSITIVE: WRONG_RECEIVER
- ❌ Token `<Method>` only in a comment → FALSE POSITIVE: COMMENT_MATCH
- ❌ Call via module import (`path`, `fs`, `Math`) → FALSE POSITIVE: MODULE_IMPORT (known limitation)

### Validated Results

During validation on a real project, the following error types were discovered and fixed:

| Error type | Before fix | After fix | Status |
|---|---|---|---|
| LOCAL_VAR_NO_TYPE (down) | 3 FP on one line (all `resolve()` from index) | 0 FP — caller_parent scoping | ✅ Fixed |
| WRONG_RECEIVER (up) | `path.resolve()` in file with `import TargetClass` | Filtered by verify_call_site_target | ✅ Fixed |
| COMMENT_MATCH (up) | Token in `//` comment matched method name | callSite shifted to real code | ✅ Fixed |
| MODULE_IMPORT (up) | `path.resolve()` where class inherits from target | receiver_type=None + inheritance → FP | ⚠️ Known limitation |

---

## Part 7: Baseline Tests (discovery-based)

Each test is described as a **template** that the LLM adapts to the specific project.

### Test B1: Direction=Down — cross-class isolation

**Goal**: Ensure that callees resolve only to the correct class.

```
Step 1: search_definitions kind=method name='ngOnInit,OnInitialized,Initialize' maxResults=10
        → select an init method in a large class with DI dependencies

Step 2: search_callers method=<init> class=<Class> direction=down depth=2

Step 3: Verify: all callees belong to <Class> or its DI services?
        There should be NO same-named methods from unrelated classes.
```

### Test B2: Direction=Down — popular method names

**Goal**: Methods with common names should not produce mass false positives.

```
Step 1: search_definitions name='resolve,get,set,execute,validate,handle' kind=method maxResults=5
        → select a class where the method is called via a local variable

Step 2: search_callers method=<method> class=<Class> direction=down depth=1

Step 3: Verify: callees are only from the local variable's class, not from all
        classes with the same method name.
```

### Test B3: Direction=Up — regression check + overload dedup

**Goal**: Ensure that fixes did not break true positives. Also verify that method overloads are not collapsed by dedup.

**Expected outcome**: PASS ✅ (OVERLOAD_DEDUP fixed — dedup keys include line_start)

```
Step 1: search_definitions kind=method maxResults=10
        → select a method with a unique name (few definitions with the same name)
        → also select a class with method overloads (same name, different signatures)

Step 2: search_callers method=<method> class=<Class> direction=up depth=1

Step 3: Verify: all callers use <Class>.<method>() via DI, inheritance, or this.
        Number of callers should be stable between runs.

Step 4 (overload check): search_definitions parent=<ClassWithOverloads> kind=method name=<overloadedName>
        → Verify: ALL overloads appear (different line_start values).
        Previously, overloads were collapsed to a single entry because dedup keys
        did not include line_start. This is now fixed.
```

### Test B4: Direction=Up — false positive elimination

**Goal**: Verify that wrong-receiver and comment matches are filtered.

```
Step 1: search_definitions name='resolve,execute,handle' kind=method maxResults=20
        → select a method in a class that has subclasses (inheritance)
        → check: is there a module/package in the project with the same method name?
          (e.g., path.resolve, Promise.resolve)

Step 2: search_callers method=<method> class=<Class> direction=up depth=1

Step 3: For each caller:
        - Read the body (search_definitions includeBody=true)
        - Confirm: call targets <Class>, not a module/package
        - Confirm: call is not in a comment

Step 4: Record FP if found.
```

### Test B5: Direction=Down — var/const type inference

**Goal**: Verify that `var x = new SomeType(); x.Method()` resolves `Method` to `SomeType`.

```
Step 1: search_grep terms='= new' ext=<ts|cs> showLines=true maxResults=5
        → find a file with the pattern: local variable = new Type(); localVar.method()

Step 2: search_callers method=<containingMethod> class=<Class> direction=down depth=1

Step 3: Verify: callee for localVar.method() resolves to Type.method(),
        not to all definitions of method() in the index.
```

### Test B6: Direction=Up — DI interface resolution (no unrelated FP)

**Goal**: Verify DI-awareness — callers via interface are found, but callers through **unrelated** interfaces with the same method name are filtered out.

**Expected outcome**: PASS ✅ (SAME_NAME_DIFFERENT_RECEIVER fixed — interface resolution filtered by relatedness + parser preserves unresolved receiver names)

```
Step 1: search_definitions kind=class baseType='I*' maxResults=10
        → find a class implementing an interface (IService → Service)

Step 2: search_definitions parent=<Service> kind=method maxResults=5
        → select a method

Step 3: search_callers method=<method> class=<Service> direction=up depth=1

Step 4: Verify: callers using IService.method() are present in results.

Step 5 (FP check): Verify: callers through UNRELATED interfaces (e.g., IUnrelatedService
        that also has a method with the same name) are NOT present in results.
        Previously, these appeared as false positives because interface resolution
        did not check relatedness. This is now fixed.
```

---

## Part 8: CI/CD Integration (future)

For full automation in the future:

1. **Fixed set of ground truth files** — 20-30 files with manual annotation
2. **Comparison script** — runs search_callers, compares with ground truth JSON
3. **Quality threshold** — precision ≥ 85%, recall ≥ 80%
4. **Regression test** — if metrics dropped compared to previous run → fail

This will enable automatic detection of regressions with every change to search-index.

---

## Changelog

- **2025-02-19**: Fixed OVERLOAD_DEDUP (dedup keys include line_start, so method overloads are no longer collapsed) and SAME_NAME_DIFFERENT_RECEIVER (interface resolution filtered by relatedness + parser preserves unresolved receiver names as `Some("varName")` instead of `None`). B3 and B6 tests now expected to PASS. Updated known limitations (added #13: local var receiver names). Added error patterns #8 and #9.
