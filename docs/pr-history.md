# Pull Request History

Repository: [pustynsky/search-index](https://github.com/pustynsky/search-index)

---

## PR #1 -- Users/pustynsky/refactor phase1 type dedup and fixes

- **Link:** [https://github.com/pustynsky/search-index/pull/1](https://github.com/pustynsky/search-index/pull/1)
- **Status:** merged
- **Created:** 2026-02-15T12:52:23Z

### Description

# Refactor: Type safety, error handling, module extraction, and binary size reduction

## What changed

This PR addresses technical debt identified during a comprehensive code review. All changes are behavior-preserving — no new features, no API changes. The goal was to improve reliability, maintainability, and correctness.

## Key improvements

### Correctness fixes

- Fixed `total_tokens` drift in incremental index updates — the counter was never decremented when files were updated or removed, causing TF-IDF scores to degrade over time
- `is_stale()` now uses `saturating_sub` to prevent underflow on clock skew, and `unwrap_or(Duration::ZERO)` instead of panicking on pre-epoch clocks

### Error handling

- Introduced `SearchError` enum (via `thiserror`) replacing `Box<dyn Error>` in all public functions
- All `cmd_*` functions now return `Result<(), SearchError>` instead of calling `process::exit(1)` — destructors run properly, functions are testable
- MCP server: stdout write errors now trigger graceful shutdown with logging instead of being silently ignored

### Type safety

- Eliminated duplicate type definitions between `lib.rs` and `main.rs` (`Posting`, `ContentIndex`, `FileEntry`, `FileIndex`, `clean_path`, `tokenize` were defined twice — one for the library, one for the binary)
- `DefinitionKind` now implements `FromStr` and `Display` traits instead of a shadowing inherent `from_str()` method
- Added `#[must_use]` on pure functions (`clean_path`, `tokenize`)

### Code organization

- Extracted `index.rs` module (268 lines) from `main.rs` — all index storage and building logic in one place
- Created `error.rs` module (104 lines) for the unified error type
- `main.rs` reduced from 2,734 to 2,009 lines (−725)
- Mutex locks in parallel walkers use `unwrap_or_else(|e| e.into_inner())` instead of `unwrap()` to prevent cascade panics

### Binary size

- Removed `tree-sitter-sequel-tsql` dependency (incompatible with tree-sitter 0.24 — version mismatch was already causing runtime warnings and SQL parsing was silently disabled)
- Binary size: **20.4 MB → 9.8 MB** (−52%)
- SQL parsing code retained for future use when a compatible grammar becomes available
- Added `strip = true` to release profile

### Quality

- Clippy warnings: 62 → 6 (55 auto-fixed, remaining 6 are intentional `too_many_arguments` in recursive call tree builders)
- Phrase search no longer reads matching files twice (content cached from verification pass)
- 11 new unit tests (+6 error handling, +3 total_tokens consistency, +4 DefinitionKind roundtrip)
- Added E2E test plan document with 24 test cases

## Testing

- **125 unit tests**, 0 failed, 0 ignored (was 114)
- **12 E2E tests** verified manually on release binary
- All existing functionality confirmed working: `find`, `index`, `fast`, `content-index`, `grep` (single/multi/regex/phrase/context/exclude), `info`, `def-index`, `serve`

## What's NOT changed

- No changes to the MCP protocol or tool schemas
- No changes to index file format (existing cached indexes still work)
- No changes to CLI argument names or defaults
- `DefaultHasher` for index paths kept as-is (migration deferred)

---

## PR #2 -- Feature/include body

- **Link:** [https://github.com/pustynsky/search-index/pull/2](https://github.com/pustynsky/search-index/pull/2)
- **Status:** merged
- **Created:** 2026-02-16T00:25:59Z

### Description

*No description provided.*

---

## PR #3 -- Substring search - tokenization

- **Link:** [https://github.com/pustynsky/search-index/pull/3](https://github.com/pustynsky/search-index/pull/3)
- **Status:** merged
- **Created:** 2026-02-16T13:22:57Z

### Description

*No description provided.*

---

## PR #4 -- feat: add --metrics flag and fix encoding in handlers.rs

- **Link:** [https://github.com/pustynsky/search-index/pull/4](https://github.com/pustynsky/search-index/pull/4)
- **Status:** merged
- **Created:** 2026-02-16T14:39:32Z

### Description

*No description provided.*

---

## PR #5 -- Benchmarks

- **Link:** [https://github.com/pustynsky/search-index/pull/5](https://github.com/pustynsky/search-index/pull/5)
- **Status:** merged
- **Created:** 2026-02-16T15:07:36Z

### Description

*No description provided.*

---

## PR #6 -- Modifier bug fix

- **Link:** [https://github.com/pustynsky/search-index/pull/6](https://github.com/pustynsky/search-index/pull/6)
- **Status:** merged
- **Created:** 2026-02-17T15:54:13Z

### Description

*No description provided.*

---

## PR #7 -- Decomposition 

- **Link:** [https://github.com/pustynsky/search-index/pull/7](https://github.com/pustynsky/search-index/pull/7)
- **Status:** merged
- **Created:** 2026-02-17T20:33:46Z

### Description

*No description provided.*

---

## PR #8 -- Fix code review issues: bounds checking, security validation, stable …

- **Link:** [https://github.com/pustynsky/search-index/pull/8](https://github.com/pustynsky/search-index/pull/8)
- **Status:** merged
- **Created:** 2026-02-17T23:30:19Z

### Description

…hash, underflow protection, monitoring

---

## PR #9 -- Add TypeScript AST parsing support and fix response truncation for se…

- **Link:** [https://github.com/pustynsky/search-index/pull/9](https://github.com/pustynsky/search-index/pull/9)
- **Status:** merged
- **Created:** 2026-02-18T07:32:18Z

### Description

…arch_definitions

---

## PR #10 -- tips updated

- **Link:** [https://github.com/pustynsky/search-index/pull/10](https://github.com/pustynsky/search-index/pull/10)
- **Status:** merged
- **Created:** 2026-02-18T09:23:38Z

### Description

*No description provided.*

---

## PR #11 -- TypeScript Call-Site Extraction for `search_callers`

- **Link:** [https://github.com/pustynsky/search-index/pull/11](https://github.com/pustynsky/search-index/pull/11)
- **Status:** merged
- **Created:** 2026-02-18T11:53:50Z

### Description

# Plan: TypeScript Call-Site Extraction for `search_callers`

> **Status:** Completed
> **Author:** Auto-generated
> **Date:** 2026-02-18
> **Depends on:** TypeScript definition support (Phase 1 — completed)

## 1. Problem Analysis

### Current State

The `search_callers` tool returns **0 results** for TypeScript methods. The definition index contains 6,844 call sites — **all from C#**. The TypeScript parser explicitly returns empty call sites:

```rust
// src/definitions/parser_typescript.rs, line 20-21
// Call sites are deferred for TypeScript — always return empty
(defs, Vec::new())
```

### Root Cause: Two Gaps

There are **two independent gaps** that must both be addressed:

#### Gap 1: TypeScript parser does not extract call sites

The C# parser ([`parser_csharp.rs`](../src/definitions/parser_csharp.rs)) has a full call-site extraction pipeline:

| C# Parser Component                                                                                | Purpose                                                                        | TypeScript Equivalent                         |
| -------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------ | --------------------------------------------- |
| [`walk_csharp_node_collecting()`](../src/definitions/parser_csharp.rs:355) collects `method_nodes` | Remembers method/constructor AST nodes for later call extraction               | **Missing** — TS walker discards method nodes |
| [`extract_call_sites()`](../src/definitions/parser_csharp.rs:148)                                  | Walks method body for invocations                                              | **Missing entirely**                          |
| [`walk_for_invocations()`](../src/definitions/parser_csharp.rs:172)                                | Recursive walker finding `invocation_expression`, `object_creation_expression` | **Missing entirely**                          |
| [`extract_invocation()`](../src/definitions/parser_csharp.rs:213)                                  | Extracts method name + receiver type from call                                 | **Missing entirely**                          |
| [`resolve_receiver_type()`](../src/definitions/parser_csharp.rs:312)                               | Maps `this`, `base`, field names → type names                                  | **Missing entirely**                          |
| Field type map (`class_field_types`)                                                               | Built from fields/properties/constructor params for receiver resolution        | **Missing entirely**                          |

#### Gap 2: Callers handler ignores `DefinitionKind::Function`

The callers handler ([`callers.rs`](../src/mcp/handlers/callers.rs)) has three places that filter by definition kind, and **none include `Function`** (the TypeScript kind for standalone functions):

| Location                                                                         | Current Filter                      | Impact                                                                               |
| -------------------------------------------------------------------------------- | ----------------------------------- | ------------------------------------------------------------------------------------ |
| [`find_containing_method()`](../src/mcp/handlers/callers.rs:209)                 | `Method \| Constructor \| Property` | TS standalone functions won't be found as "containing method" for a call site        |
| [`build_caller_tree()`](../src/mcp/handlers/callers.rs:373) definition_locations | `Method \| Constructor`             | TS function definitions won't be excluded from "callers" (would show self as caller) |
| [`build_callee_tree()`](../src/mcp/handlers/callers.rs:565)                      | `Method \| Constructor`             | TS functions won't be found as callee targets                                        |
| [`resolve_call_site()`](../src/mcp/handlers/callers.rs:704)                      | `Method \| Constructor`             | TS functions won't be resolved as call targets                                       |

---

## Step 0: Safety — C# Non-Regression

> **Principle: "First, do no harm."**
> Every change must be verified against existing C# caller tests before anything else.
> The C# call analyzer is production-critical. No TypeScript feature is worth breaking it.

### 0a. Baseline Snapshot

Before starting implementation, run:

```bash
cargo test --bin search
```

Record the **exact number of passing C# caller tests**. This is the baseline that must **never decrease** throughout the entire implementation.

### 0b. Isolation Strategy

| Rule                                          | Detail                                                                                                                                                                                                                   |
| --------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **Do NOT modify `parser_csharp.rs`**          | The C# parser ([`parser_csharp.rs`](../src/definitions/parser_csharp.rs)) must not be touched at all — not even formatting or refactoring                                                                                |
| **TS extraction is self-contained**           | All TypeScript call-site extraction goes entirely into [`parser_typescript.rs`](../src/definitions/parser_typescript.rs)                                                                                                 |
| **Callers handler changes are additive only** | The changes to [`callers.rs`](../src/mcp/handlers/callers.rs) (adding `Function` kind) must be purely additive — existing `Method \| Constructor \| Property` filters remain unchanged, `Function` is only ADDED to them |
| **Shared code paths need dual testing**       | Any code path in shared modules (e.g., [`callers.rs`](../src/mcp/handlers/callers.rs), [`types.rs`](../src/definitions/types.rs)) must be tested with both C# and TypeScript inputs                                      |

### 0c. Mandatory Verification After Each Step

After **every single implementation step** (Steps 1–9):

1. Run `cargo test --bin search`
2. Confirm **ALL existing C# caller tests still pass**
3. Compare the passing test count against the baseline from Step 0a
4. **If any C# test fails → STOP immediately and fix before proceeding**

### 0d. C# Regression Test

Add at least one **explicit C# regression test** that exercises the C# caller path end-to-end. This test should:

- Use a representative C# code snippet with method calls, constructor calls, and field-based receiver resolution
- Verify that `search_callers` with `direction: "up"` and `direction: "down"` both return correct results
- Be added **before** any shared code is modified (i.e., before Steps 5–6)
- Serve as a canary: if this test ever fails during TS implementation, something went wrong in shared code

---

## Pre-Implementation Fixes

> These issues were discovered during impact analysis and **must be resolved before** starting the implementation steps below.

### Fix A: `.tsx` Grammar Bug (CRITICAL — must fix first)

[`mod.rs`](../src/definitions/mod.rs:127) (line ~127) uses `LANGUAGE_TYPESCRIPT` for **both** `.ts` and `.tsx` files during batch indexing. However, [`incremental.rs`](../src/definitions/incremental.rs:48) (lines ~48-52) correctly uses `LANGUAGE_TSX` for `.tsx` files.

This means batch indexing and incremental indexing produce **different ASTs** for `.tsx` files — batch indexing parses `.tsx` with the wrong grammar.

**Fix:** Update batch indexing in [`mod.rs`](../src/definitions/mod.rs:127) to use `LANGUAGE_TSX` for `.tsx` files, matching the incremental behavior.

**Why this is critical:** This must be done **BEFORE** adding call-site extraction. If `.tsx` files are parsed with the wrong grammar, call-site extraction will produce garbage results for any `.tsx` file. Since `.tsx` is the dominant file extension in React codebases, this would silently corrupt a large portion of the index.

### Fix B: Arrow Function Class Properties

TypeScript commonly defines methods as arrow function class properties:

```typescript
class MyService {
  processItem = (item: Item): void => {
    this.validate(item);
  };
}
```

In tree-sitter, this is a `public_field_definition`, **not** a `method_definition`. Step 1 of the plan currently only collects `method_definition` and `function_declaration` nodes.

**Amendment:** Step 1 must **ALSO** collect `public_field_definition` nodes where the initializer is an `arrow_function`, and extract call sites from their bodies. The detection logic is:

1. Node kind is `public_field_definition`
2. The node has a child with field name `value`
3. That child's kind is `arrow_function`

If all three conditions are met, treat the `arrow_function` body as a method body for call-site extraction purposes.

---

## 2. TypeScript Call Patterns to Recognize

### 2.1 Method Calls (member access)

```typescript
// tree-sitter node: call_expression > member_expression
this.userService.getUser(id); // receiver: this.userService → field type
this.processItem(item); // receiver: this → class name
super.initialize(); // receiver: super → base class
someService.doWork(); // receiver: someService → field type
UserService.staticMethod(); // receiver: UserService → class name (uppercase)
```

**tree-sitter AST structure:**

```
(call_expression
  function: (member_expression
    object: (this) | (identifier) | (member_expression)
    property: (property_identifier))    ← method name
  arguments: (arguments))
```

### 2.2 Simple Function Calls

```typescript
// tree-sitter node: call_expression > identifier
processOrder(order); // no receiver
await fetchData(url); // no receiver, wrapped in await
```

**tree-sitter AST structure:**

```
(call_expression
  function: (identifier)               ← function name
  arguments: (arguments))
```

### 2.3 Constructor Calls (new expressions)

```typescript
// tree-sitter node: new_expression
const svc = new UserService(logger);
throw new ValidationError("invalid");
```

**tree-sitter AST structure:**

```
(new_expression
  constructor: (identifier)            ← class name
  arguments: (arguments))
```

### 2.4 Optional Chaining Calls

```typescript
// tree-sitter node: call_expression > member_expression with optional_chain
this.service?.getUser(id);
obj?.method();
```

**tree-sitter AST structure:**

```
(call_expression
  function: (member_expression
    object: (member_expression ...)
    property: (property_identifier))    ← method name
  arguments: (arguments))
```

Note: In tree-sitter-typescript, optional chaining (`?.`) is represented within the member_expression with an `optional_chain` token. The method name extraction works the same way.

### 2.5 Chained Calls

```typescript
// These produce nested call_expression nodes
this.getItems()
  .filter((x) => x.active)
  .map((x) => x.id);
```

Each `.filter()` and `.map()` is a separate `call_expression`. We extract each independently — no need for special chaining logic (same as C#).

### 2.6 Patterns NOT in Scope (Phase 2+)

| Pattern                                                          | Why Deferred                                   |
| ---------------------------------------------------------------- | ---------------------------------------------- |
| Dynamic calls: `obj[methodName]()`                               | Receiver type unknowable statically            |
| Callback invocations: `callback()` where callback is a parameter | Would need type flow analysis                  |
| Import-resolved module calls: `import { foo } from './module'`   | Requires import resolution                     |
| Decorator calls: `@Injectable()`                                 | Already captured as attributes, not call sites |
| Tagged template literals: `` html`...` ``                        | Rare, specialized                              |

---

## 3. Implementation Steps

### Step 1: Add `method_nodes` collection to TS walker

**File:** [`src/definitions/parser_typescript.rs`](../src/definitions/parser_typescript.rs)

Change [`walk_typescript_node_collecting()`](../src/definitions/parser_typescript.rs:26) signature to accept a `method_nodes` parameter (same pattern as C#'s [`walk_csharp_node_collecting()`](../src/definitions/parser_csharp.rs:355)):

```rust
fn walk_typescript_node_collecting<'a>(
    node: tree_sitter::Node<'a>,
    source: &str,
    file_id: u32,
    parent_name: Option<&str>,
    defs: &mut Vec<DefinitionEntry>,
    method_nodes: &mut Vec<(usize, tree_sitter::Node<'a>)>,  // NEW
)
```

For `method_definition`, `function_declaration`, and `public_field_definition` (arrow function class properties — see Fix B) nodes, after pushing the def, also push the node reference:

```rust
"method_definition" => {
    if let Some(def) = extract_ts_method_def(node, source, file_id, parent_name) {
        let idx = defs.len();
        defs.push(def);
        method_nodes.push((idx, node));  // NEW
        return;
    }
}
"function_declaration" => {
    if let Some(def) = extract_ts_function_def(node, source, file_id, parent_name) {
        let idx = defs.len();
        defs.push(def);
        method_nodes.push((idx, node));  // NEW
        return;
    }
}
"public_field_definition" => {
    // Arrow function class properties (see Fix B in Pre-Implementation Fixes)
    // Only collect if the initializer is an arrow_function
    if let Some(value_node) = node.child_by_field_name("value") {
        if value_node.kind() == "arrow_function" {
            if let Some(def) = extract_ts_method_def(node, source, file_id, parent_name) {
                let idx = defs.len();
                defs.push(def);
                method_nodes.push((idx, value_node));  // Push the arrow_function node
                return;
            }
        }
    }
}
```

### Step 2: Build field type maps for TypeScript classes

**File:** [`src/definitions/parser_typescript.rs`](../src/definitions/parser_typescript.rs)

After collecting all definitions, build `class_field_types` from:

1. **Class fields** (`public_field_definition`) — extract type from `: TypeAnnotation`
2. **Constructor parameters** with visibility modifiers (`constructor(private userService: IUserService)`) — these implicitly create class fields in TypeScript

TypeScript-specific: Constructor parameter properties (e.g., `private readonly service: IService`) are auto-promoted to fields. Parse the constructor's `formal_parameters` to find parameters with `accessibility_modifier` children and extract their type annotations.

```rust
// Build per-class field type maps
let mut class_field_types: HashMap<String, HashMap<String, String>> = HashMap::new();

for def in &defs {
    if let Some(ref parent) = def.parent {
        if def.kind == DefinitionKind::Field {
            if let Some(ref sig) = def.signature {
                // Parse "fieldName: TypeName" from signature
                if let Some(type_name) = extract_ts_field_type(sig) {
                    class_field_types
                        .entry(parent.clone())
                        .or_default()
                        .insert(def.name.clone(), type_name);
                }
            }
        }
    }
}
```

Also extract constructor parameter types:

```rust
for def in &defs {
    if def.kind == DefinitionKind::Constructor {
        if let Some(ref parent) = def.parent {
            if let Some(ref sig) = def.signature {
                let param_types = extract_ts_constructor_param_types(sig);
                let field_map = class_field_types.entry(parent.clone()).or_default();
                for (param_name, param_type) in param_types {
                    field_map.entry(param_name).or_insert(param_type);
                }
            }
        }
    }
}
```

### Step 3: Implement call-site extraction functions

**File:** [`src/definitions/parser_typescript.rs`](../src/definitions/parser_typescript.rs)

Add the following functions (mirroring the C# equivalents):

#### `extract_ts_call_sites()`

```rust
fn extract_ts_call_sites(
    method_node: tree_sitter::Node,
    source: &str,
    class_name: &str,
    field_types: &HashMap<String, String>,
    base_types: &[String],
) -> Vec<CallSite>
```

Finds the method body (`statement_block` for methods/functions, or `arrow_function` body), then calls `walk_ts_for_calls()`.

#### `walk_ts_for_calls()`

Recursive walker. Matches on:

| tree-sitter Node Kind | Action                                    |
| --------------------- | ----------------------------------------- |
| `call_expression`     | Extract via `extract_ts_call()`           |
| `new_expression`      | Extract via `extract_ts_new_expression()` |
| Other                 | Recurse into children                     |

#### `extract_ts_call()`

Examines the `function` child of `call_expression`:

| Child Kind          | Method Name Source                     | Receiver Resolution                           |
| ------------------- | -------------------------------------- | --------------------------------------------- |
| `identifier`        | The identifier text                    | None (bare function call)                     |
| `member_expression` | `property` field (property_identifier) | `object` field → `resolve_ts_receiver_type()` |

#### `extract_ts_new_expression()`

Examines the `constructor` child of `new_expression`:

```rust
fn extract_ts_new_expression(node: tree_sitter::Node, source: &str) -> Option<CallSite> {
    let constructor_node = node.child_by_field_name("constructor")?;
    let type_name = node_text(constructor_node, source);
    Some(CallSite {
        method_name: type_name.to_string(),
        receiver_type: Some(type_name.to_string()),
        line: node.start_position().row as u32 + 1,
    })
}
```

#### `resolve_ts_receiver_type()`

Maps receiver expressions to type names:

| Receiver Kind/Text                             | Resolution                                                       |
| ---------------------------------------------- | ---------------------------------------------------------------- |
| `this` / `this_expression`                     | Current class name                                               |
| `super` / `super_expression`                   | First base type                                                  |
| Identifier matching a field name               | Field type from `class_field_types` map                          |
| Identifier starting with uppercase             | Assume class/static call → use as type name                      |
| `member_expression` (e.g., `this.userService`) | Resolve recursively: `this` → class, `.userService` → field type |
| Other                                          | `None`                                                           |

### Step 4: Wire call extraction into `parse_typescript_definitions()`

**File:** [`src/definitions/parser_typescript.rs`](../src/definitions/parser_typescript.rs)

Update [`parse_typescript_definitions()`](../src/definitions/parser_typescript.rs:7) to:

1. Pass `method_nodes` to the walker
2. Build field type maps
3. Iterate over `method_nodes` and call `extract_ts_call_sites()` for each
4. Return the collected call sites instead of `Vec::new()`

```rust
pub(crate) fn parse_typescript_definitions(
    parser: &mut tree_sitter::Parser,
    source: &str,
    file_id: u32,
) -> (Vec<DefinitionEntry>, Vec<(usize, Vec<CallSite>)>) {
    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return (Vec::new(), Vec::new()),
    };

    let mut defs = Vec::new();
    let source_bytes = source; // TS parser uses &str
    let mut method_nodes: Vec<(usize, tree_sitter::Node)> = Vec::new();
    walk_typescript_node_collecting(tree.root_node(), source_bytes, file_id, None, &mut defs, &mut method_nodes);

    // Build field type maps and base type maps
    let class_field_types = build_ts_class_field_types(&defs);
    let class_base_types = build_ts_class_base_types(&defs);

    // Extract call sites from method nodes
    let mut call_sites: Vec<(usize, Vec<CallSite>)> = Vec::new();
    for &(def_local_idx, method_node) in &method_nodes {
        let def = &defs[def_local_idx];
        let parent_name = def.parent.as_deref().unwrap_or("");
        let field_types = class_field_types.get(parent_name)
            .cloned().unwrap_or_default();
        let base_types = class_base_types.get(parent_name)
            .cloned().unwrap_or_default();

        let calls = extract_ts_call_sites(method_node, source, parent_name, &field_types, &base_types);
        if !calls.is_empty() {
            call_sites.push((def_local_idx, calls));
        }
    }

    (defs, call_sites)
}
```

### Step 5: Fix callers handler to support `DefinitionKind::Function`

> **⚠️ SHARED CODE — verify C# tests after this step.**
> This step modifies [`callers.rs`](../src/mcp/handlers/callers.rs), which is shared between C# and TypeScript.
> After completing this step, run `cargo test --bin search` and confirm ALL existing C# caller tests still pass before proceeding.

**File:** [`src/mcp/handlers/callers.rs`](../src/mcp/handlers/callers.rs)

Four locations need updating:

#### 5a. [`find_containing_method()`](../src/mcp/handlers/callers.rs:209) — line 209

```rust
// BEFORE:
DefinitionKind::Method | DefinitionKind::Constructor | DefinitionKind::Property => {}

// AFTER:
DefinitionKind::Method | DefinitionKind::Constructor | DefinitionKind::Property | DefinitionKind::Function => {}
```

#### 5b. [`build_caller_tree()`](../src/mcp/handlers/callers.rs:373) definition_locations — line 373

```rust
// BEFORE:
&& (def.kind == DefinitionKind::Method || def.kind == DefinitionKind::Constructor) {

// AFTER:
&& (def.kind == DefinitionKind::Method || def.kind == DefinitionKind::Constructor || def.kind == DefinitionKind::Function) {
```

#### 5c. [`build_callee_tree()`](../src/mcp/handlers/callers.rs:565) — line 565

```rust
// BEFORE:
let kind_ok = d.kind == DefinitionKind::Method || d.kind == DefinitionKind::Constructor;

// AFTER:
let kind_ok = d.kind == DefinitionKind::Method || d.kind == DefinitionKind::Constructor || d.kind == DefinitionKind::Function;
```

#### 5d. [`resolve_call_site()`](../src/mcp/handlers/callers.rs:704) — line 704

```rust
// BEFORE:
if def.kind != DefinitionKind::Method && def.kind != DefinitionKind::Constructor {

// AFTER:
if def.kind != DefinitionKind::Method && def.kind != DefinitionKind::Constructor && def.kind != DefinitionKind::Function {
```

### Step 6: Update ambiguity warning in callers handler

> **⚠️ SHARED CODE — verify C# tests after this step.**
> This step modifies [`callers.rs`](../src/mcp/handlers/callers.rs), which is shared between C# and TypeScript.
> After completing this step, run `cargo test --bin search` and confirm ALL existing C# caller tests still pass before proceeding.

**File:** [`src/mcp/handlers/callers.rs`](../src/mcp/handlers/callers.rs)

The ambiguity check at line 69 also filters by `Method | Constructor`. Add `Function`:

```rust
// BEFORE:
.filter(|d| d.kind == DefinitionKind::Method || d.kind == DefinitionKind::Constructor)

// AFTER:
.filter(|d| d.kind == DefinitionKind::Method || d.kind == DefinitionKind::Constructor || d.kind == DefinitionKind::Function)
```

### Step 7: Add unit tests for TypeScript call-site extraction

**File:** [`src/definitions/definitions_tests.rs`](../src/definitions/definitions_tests.rs)

Add tests covering each TypeScript call pattern:

| Test Name                                 | Input                                                   | Expected Call Sites                                                           |
| ----------------------------------------- | ------------------------------------------------------- | ----------------------------------------------------------------------------- |
| `test_ts_call_sites_simple_function_call` | `function foo() { bar(); }`                             | `CallSite { method_name: "bar", receiver_type: None }`                        |
| `test_ts_call_sites_method_call`          | Class with `this.service.getUser(id)`                   | `CallSite { method_name: "getUser", receiver_type: Some("IUserService") }`    |
| `test_ts_call_sites_this_call`            | Class with `this.process()`                             | `CallSite { method_name: "process", receiver_type: Some("MyClass") }`         |
| `test_ts_call_sites_super_call`           | Class with `super.init()`                               | `CallSite { method_name: "init", receiver_type: Some("BaseClass") }`          |
| `test_ts_call_sites_new_expression`       | `const x = new UserService()`                           | `CallSite { method_name: "UserService", receiver_type: Some("UserService") }` |
| `test_ts_call_sites_static_call`          | `UserService.create()`                                  | `CallSite { method_name: "create", receiver_type: Some("UserService") }`      |
| `test_ts_call_sites_chained`              | `this.getItems().filter(...)`                           | Two call sites: `getItems` + `filter`                                         |
| `test_ts_call_sites_constructor_di`       | `constructor(private svc: IService) { ... svc.call() }` | `CallSite { receiver_type: Some("IService") }`                                |
| `test_ts_call_sites_optional_chaining`    | `this.service?.getUser(id)`                             | `CallSite { method_name: "getUser" }`                                         |
| `test_ts_call_sites_await`                | `await this.fetchData()`                                | `CallSite { method_name: "fetchData" }`                                       |
| `test_ts_call_sites_standalone_function`  | Top-level `function process() { helper(); }`            | `CallSite { method_name: "helper", receiver_type: None }`                     |

### Step 8: Update documentation

**Files:**

- [`README.md`](../README.md) — update `search_callers` description to mention TypeScript support
- [`docs/e2e-test-plan.md`](../docs/e2e-test-plan.md) — add TypeScript caller scenarios
- [`docs/typescript-support-plan.md`](../docs/typescript-support-plan.md) — update Phase 2 status

### Step 9: Run tests and reinstall

1. Run `cargo test --bin search` — verify 0 failures
2. Ask user to stop MCP server
3. Run `cargo install --path . --force`

---

## 4. Architecture: Data Flow with TypeScript Callers

```mermaid
flowchart TB
    subgraph Index Build
        A[.ts/.tsx file] --> B[parse_typescript_definitions]
        B --> C[walk_typescript_node_collecting]
        C --> D[defs + method_nodes]
        D --> E[build field type maps]
        E --> F[extract_ts_call_sites per method]
        F --> G[Vec of CallSite per method]
        G --> H[method_calls HashMap in DefinitionIndex]
    end

    subgraph search_callers - direction up
        I[method name] --> J[content index grep for token]
        J --> K[find_containing_method - needs Function kind]
        K --> L[build caller tree recursively]
    end

    subgraph search_callers - direction down
        M[method name] --> N[find definition in name_index]
        N --> O[lookup method_calls for def idx]
        O --> P[resolve_call_site - needs Function kind]
        P --> Q[build callee tree recursively]
    end
```

### How `direction: "up"` Works (grep-based — no call-site data needed)

The "up" direction uses the **content index** (grep) to find where a method name token appears, then uses [`find_containing_method()`](../src/mcp/handlers/callers.rs:199) to determine which function/method contains that reference. This approach **already works for TypeScript** once Gap 2 is fixed (adding `Function` to kind filters). No call-site extraction is needed for "up".

### How `direction: "down"` Works (AST call-site data required)

The "down" direction uses the **pre-computed `method_calls`** map from AST analysis. This is where Gap 1 matters — without call-site extraction, `method_calls` has no TypeScript entries, so "down" returns nothing.

### Implication

**Gap 2 alone** (adding `Function` to kind filters) is sufficient to make `direction: "up"` work for TypeScript. **Both gaps** must be fixed for `direction: "down"`.

---

## 5. Differences from C# That Affect Implementation

| Aspect               | C#                                                            | TypeScript                                             | Impact                                       |
| -------------------- | ------------------------------------------------------------- | ------------------------------------------------------ | -------------------------------------------- |
| Call node type       | `invocation_expression`                                       | `call_expression`                                      | Different tree-sitter node names             |
| Constructor call     | `object_creation_expression`                                  | `new_expression`                                       | Different node names                         |
| Member access        | `member_access_expression`                                    | `member_expression`                                    | Different node names                         |
| Conditional access   | `conditional_access_expression` + `member_binding_expression` | `member_expression` with optional_chain                | Simpler in TS                                |
| Method body          | `block` or `arrow_expression_clause`                          | `statement_block`                                      | Different node names                         |
| DI pattern           | Constructor injection + `_field`                              | Constructor parameter properties (`private svc: ISvc`) | TS auto-promotes to fields                   |
| Standalone functions | Does not exist (everything in classes)                        | `function_declaration` at module level                 | New kind: `Function` — needs handler support |
| Source type          | `&[u8]`                                                       | `&str`                                                 | TS parser already uses `&str`                |
| Generic calls        | `generic_name`                                                | Type arguments in call_expression                      | Similar handling                             |

---

## 6. Receiver Type Resolution Strategy

TypeScript receiver resolution is simpler than C# because:

- No `m_` prefix convention for fields
- Constructor parameter properties are explicit with type annotations
- `this.fieldName` is the standard pattern (no bare `fieldName` for fields)

### Resolution Table

| Receiver Expression  | Resolution Strategy                              |
| -------------------- | ------------------------------------------------ |
| `this`               | Current class name (from `parent_name`)          |
| `super`              | First base type from heritage                    |
| `this.fieldName`     | Look up `fieldName` in class field type map      |
| `ClassName.method()` | Use `ClassName` directly (starts with uppercase) |
| `localVar.method()`  | `None` (no type flow analysis)                   |
| `(expr).method()`    | `None` (expression too complex)                  |

---

## 7. Known Limitations

> These limitations are accepted for the initial implementation. They represent trade-offs between implementation complexity and practical value.

### L1: Standalone Function False Positives

TypeScript standalone functions have `receiver_type: None` in their call sites. The resolver accepts **any** matching method name across the entire codebase when there is no receiver type to constrain the search.

Common names like `handleClick` appearing in 50 files will produce 50 ambiguous results. This is inherent to the grep-based "up" direction and the untyped "down" direction for bare function calls.

**Mitigation:** The existing `ext` parameter in [`search_callers`](../src/mcp/handlers/callers.rs) allows filtering to TS-only results (e.g., `"ext": "ts,tsx"`). Additionally, the ambiguity warning system already alerts users when multiple definitions match a method name.

### L2: No Import Resolution

```typescript
import { process } from "./orderService";
process(order);
```

This produces `CallSite { method_name: "process", receiver_type: None }`. Without knowing the import source, this matches **every** `process` function in the index.

Proper import resolution would require building a module dependency graph, which is deferred to a future phase.

### L3: Dynamic/Computed Calls Not Supported

The following patterns are **not** extracted as call sites:

- **Computed property access:** `obj[methodName]()`
- **Callback invocations:** `callback()` where `callback` is a parameter or variable
- **`bind`/`call`/`apply` patterns:** `fn.call(thisArg, ...)`, `fn.apply(thisArg, [...])`
- **Destructured methods:** `const { getUser } = this.service; getUser(id);` — the destructuring loses the receiver type, so `getUser` becomes an unresolvable bare function call

These patterns require type flow analysis and are out of scope for the initial implementation.

### L4: Language Filtering

The existing `ext` parameter in [`search_callers`](../src/mcp/handlers/callers.rs) already provides language filtering at query time. No separate index per language is needed.

Example: `{ "method": "getUser", "ext": "ts,tsx" }` returns only TypeScript callers, excluding any C# methods with the same name.

This is sufficient for the common case where a user wants callers within a specific language ecosystem.

---

## 8. Risk Assessment

| Risk                                                          | Likelihood | Impact | Mitigation                                                                         |
| ------------------------------------------------------------- | ---------- | ------ | ---------------------------------------------------------------------------------- |
| tree-sitter-typescript AST node names differ from expected    | Medium     | Medium | Verify with `tree-sitter playground` or `node.kind()` debugging; add AST dump test |
| Optional chaining AST representation unclear                  | Medium     | Low    | Test with actual tree-sitter parse; graceful fallback to `None` receiver           |
| Arrow functions as method bodies                              | Low        | Low    | Find `statement_block` OR expression body                                          |
| Performance impact from call extraction on large TS codebases | Low        | Low    | Same parallel parsing infra; call extraction is O(n) per method body               |
| `Function` kind added to callers handler breaks C# behavior   | Very Low   | High   | `Function` kind doesn't exist in C# defs, so the extra filter is a no-op for C#    |
| TS files with JSX syntax in `.tsx` have different AST         | Low        | Low    | tree-sitter-typescript handles TSX natively; call expression nodes same            |

---

## 9. Test Strategy

### Unit Tests (Phase 1 — in `definitions_tests.rs`)

- **11 call-pattern tests** (as detailed in Step 7)
- **1 integration test:** Parse a realistic TS class with DI, verify call sites have correct `method_name` and `receiver_type`
- **1 negative test:** Verify decorator calls `@Injectable()` are NOT extracted as call sites

### Unit Tests (Phase 1 — in `handlers_tests.rs`)

- **1 test:** Verify `find_containing_method()` finds a `Function` kind definition
- **1 test:** Verify `resolve_call_site()` resolves to a `Function` kind definition

### E2E Tests (in `docs/e2e-test-plan.md`)

- `search_callers` with `method: "someFunction"` on a TS codebase, `direction: "up"` — returns callers
- `search_callers` with `method: "someMethod"`, `class: "MyService"`, `direction: "down"` — returns callees with receiver types
- Mixed C#/TS project: callers work for both languages independently

---

## 10. Implementation Order Rationale

The steps are ordered to deliver value incrementally:

1. **Steps 5-6 first** (callers handler kind filter fix) — this alone enables `direction: "up"` for TypeScript with zero parser changes, since the "up" direction uses content index grep, not AST call sites.

2. **Steps 1-4** (TS call-site extraction) — enables `direction: "down"` for TypeScript.

3. **Steps 7-9** (tests, docs, install) — verification and documentation.

This means a partial deployment is possible: fix the kind filters first, ship, then add call-site extraction later.


---

## PR #12 -- cap search-callers

- **Link:** [https://github.com/pustynsky/search-index/pull/12](https://github.com/pustynsky/search-index/pull/12)
- **Status:** merged
- **Created:** 2026-02-18T12:47:38Z

### Description

*No description provided.*

---

## PR #13 -- fix: use lossy UTF-8 file reading to prevent silent indexing failures…

- **Link:** [https://github.com/pustynsky/search-index/pull/13](https://github.com/pustynsky/search-index/pull/13)
- **Status:** merged
- **Created:** 2026-02-18T14:09:27Z

### Description

# Changelog — Lossy UTF-8 File Reading

## Summary

Fixed a critical bug where files containing non-UTF8 bytes (e.g., Windows-1252 encoded smart quotes in comments) were **silently skipped** during definition and content indexing. A single byte like `0x92` (right single quote `'`) in a comment on line 713 of a 745-line file would cause the entire file to be dropped from the index with zero definitions extracted and no error message.

**Root cause:** `std::fs::read_to_string()` requires strict valid UTF-8 and returns `Err` for any non-UTF8 byte. The error handler silently incremented an error counter without logging the file path.

**Fix:** Replace `read_to_string` with `std::fs::read()` + `String::from_utf8_lossy()` in all indexing paths. Non-UTF8 bytes are replaced with the Unicode replacement character `U+FFFD` (`�`), and a warning is logged with the file path. This preserves all valid content while gracefully handling encoding issues.

**Total: 11 files changed, ~100 insertions.**

---

## Bug Fix

### `src/lib.rs`
- **New function:** `read_file_lossy(path) -> io::Result<(String, bool)>` — reads a file with lossy UTF-8 conversion. Returns `(content, was_lossy)` flag.

### `src/definitions/mod.rs` (definition index build)
- **What:** Replaced `std::fs::read_to_string()` with `read_file_lossy()`.
- **Why:** Files with non-UTF8 bytes (e.g., Windows-1252 `0x92` in comments) were silently skipped, producing 0 definitions for otherwise valid source files.
- **Added:** Warning log for each lossy file, and `lossy_file_count` tracking in build output.

### `src/definitions/incremental.rs` (file watcher incremental update)
- **What:** Same `read_to_string` → `read_file_lossy` replacement.
- **Why:** Live file watcher updates had the same bug — editing a non-UTF8 file would silently fail to update definitions.

### `src/index.rs` (content index build)
- **What:** Same fix for content index (used by `search_grep`).
- **Why:** Non-UTF8 files were also silently excluded from the content search index.

### `src/mcp/watcher.rs` (content watcher)
- **What:** Same fix for content watcher incremental updates.

---

## Diagnostics

### `src/definitions/types.rs`
- **New fields** on `DefinitionIndex`: `parse_errors: usize` (files that couldn't be read at all) and `lossy_file_count: usize` (files read with lossy conversion).
- Both fields have `#[serde(default)]` for backward compatibility with existing `.didx` files.

### `src/definitions/parser_csharp.rs` / `src/definitions/parser_typescript.rs`
- **Added:** Warning log when `parser.parse()` returns `None` (tree-sitter internal failure), including the `file_id` for debugging.

### `src/cli/info.rs` (`search_info` response)
- **Added:** `readErrors` and `lossyUtf8Files` fields in definition index info (only shown when > 0).

### `src/mcp/handlers/definitions.rs` (`search_definitions` summary)
- **Added:** `readErrors` and `lossyUtf8Files` fields in query summary (only shown when > 0).

---

## Tests

### Unit Tests (in `src/definitions/definitions_tests.rs`)
- `test_parse_csharp_with_non_utf8_byte_in_comment` — Verifies that a C# file with a `0x92` byte in a comment still parses correctly after lossy conversion. Checks that class, method, and constructor definitions are all extracted.
- `test_read_file_lossy_with_valid_utf8` — Verifies `read_file_lossy` returns `was_lossy=false` for a valid UTF-8 file.
- `test_read_file_lossy_with_non_utf8_byte` — Verifies `read_file_lossy` returns `was_lossy=true` and content with replacement character for a file with `0x92` byte.

### E2E Test (in `docs/e2e-test-plan.md`)
- **T-LOSSY:** Creates a `.cs` file with a `0x92` byte, runs `def-index`, and verifies definitions are extracted and a warning is logged.

---

## PR #14 -- perf: lazy parsers + parallel tokenization (3.6× faster index build)

- **Link:** [https://github.com/pustynsky/search-index/pull/14](https://github.com/pustynsky/search-index/pull/14)
- **Status:** merged
- **Created:** 2026-02-18T17:35:45Z

### Description

## perf: lazy parsers + parallel tokenization (3.6× faster index build)

### Problem

Index building regressed from ~50s to ~150s after TypeScript support was added:
- **Definition index:** TypeScript grammars eagerly loaded on every thread for C#-only projects (+65s)
- **Content index:** Tokenization of 57M tokens across 65K files was single-threaded (~44s)

### Changes

#### 1. Lazy parser initialization (`src/definitions/mod.rs`)
- TS/TSX parsers use `Option<Parser>` + `get_or_insert_with()` — created only when a thread encounters a `.ts`/`.tsx` file
- SQL parser removed from thread workers (parsing disabled)
- For C#-only repos, zero TypeScript grammar overhead

#### 2. Extension-aware `def_exts` filtering (`src/cli/serve.rs`)
- `def_exts` computed as intersection of `--ext` flag and supported languages (`cs`, `ts`, `tsx`)
- Previously hardcoded to `"cs,sql,ts,tsx"` — parsed all extensions regardless of `--ext`

#### 3. Parallel content tokenization (`src/index.rs`)
- Replaced serial tokenization loop with `std::thread::scope` chunked parallelism
- Each thread builds a local `HashMap<String, Vec<Posting>>` — no shared mutable state
- Sequential merge step combines thread-local results (~50ms)

### Benchmarks (65K files, 24-core CPU)

| Index | Before (regression) | After | Speedup |
|-------|-------------------|-------|---------|
| `.cidx` (content) | ~50s | **22s** | 2.3× |
| `.didx` (definitions) | ~100s | **19.5s** | 5.1× |
| **Total** | **~150s** | **42s** | **3.6×** |

### Tests

- 3 new unit tests for extension filtering (`test_build_def_index_cs_only_no_ts_parsers`, `test_build_def_index_cs_and_ts`, `test_build_def_index_ts_only`)
- All 258 tests pass, 0 failures

### Documentation updated

- `docs/concurrency.md` — Content Index Build section rewritten (parallel tokenization), Definition Index Build updated (lazy parsers)
- `docs/architecture.md` — DefinitionIndex: "C# only" → "C# and TypeScript/TSX"
- `docs/e2e-test-plan.md` — T60 added (extension filtering), duplicate T45 renumbered
- `docs/changelog-lazy-parsers.md` — full changelog with benchmarks
- `README.md` — parallel tokenization added to features list


---

## PR #15 -- Add LZ4 frame compression for index files (.idx, .cidx, .didx)

- **Link:** [https://github.com/pustynsky/search-index/pull/15](https://github.com/pustynsky/search-index/pull/15)
- **Status:** merged
- **Created:** 2026-02-18T18:03:58Z

### Description

# PR: Add LZ4 frame compression for index files (.idx, .cidx, .didx)

## Summary

Add LZ4 frame compression for all index files on disk, reducing total index size by ~42% (566 MB → 327 MB) with minimal impact on load/save time.

## Changes

### Core implementation

- Add `lz4_flex = "0.11"` dependency (pure Rust, no C dependency)
- Add shared [`save_compressed()`](../src/index.rs:25) / [`load_compressed()`](../src/index.rs:57) helpers in `src/index.rs`
- All compressed files start with 4-byte magic `LZ4S` for format identification
- Backward compatible: [`load_compressed`](../src/index.rs:57) auto-detects LZ4 vs legacy uncompressed format
- Streaming compression via `FrameEncoder`/`FrameDecoder` — no intermediate full buffer in memory
- Compression ratio and timing logged to stderr on save/load

### Files modified

| File                                                          | What changed                                                                                                                           |
| ------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| [`Cargo.toml`](../Cargo.toml)                                 | Added `lz4_flex = "0.11"`                                                                                                              |
| [`src/index.rs`](../src/index.rs)                             | `LZ4_MAGIC`, `save_compressed()`, `load_compressed()`, updated FileIndex/ContentIndex save/load, updated `read_root_from_index_file()` |
| [`src/definitions/storage.rs`](../src/definitions/storage.rs) | Updated DefinitionIndex save/load to use compression helpers                                                                           |
| [`src/cli/info.rs`](../src/cli/info.rs)                       | Updated to use `load_compressed` for index deserialization                                                                             |
| [`docs/storage.md`](storage.md)                               | Serialization format updated with LZ4 details                                                                                          |
| [`docs/architecture.md`](architecture.md)                     | LZ4 compression details added to index properties                                                                                      |
| [`docs/benchmarks.md`](benchmarks.md)                         | All disk sizes and load times updated with measured values                                                                             |
| [`docs/e2e-test-plan.md`](e2e-test-plan.md)                   | T-LZ4 test case added                                                                                                                  |
| [`README.md`](../README.md)                                   | LZ4 compression added to features list                                                                                                 |

### Tests

- 4 new unit tests:
  - `test_save_load_compressed_roundtrip` — save with LZ4, load back, verify data matches
  - `test_load_compressed_legacy_uncompressed` — write raw bincode, load with `load_compressed` → backward compat
  - `test_load_compressed_missing_file_returns_none` — non-existent file returns `None`
  - `test_compressed_file_smaller_than_uncompressed` — compressed file is smaller than raw bincode
- T-LZ4 added to E2E test plan
- All 259 tests pass

## Measured Results

Benchmark on C:\Repos\Shared (53,900 files, extensions: cs, sql, ts):

| Index              | Uncompressed | Compressed   | Ratio    | Save Overhead | Load Time |
| ------------------ | ------------ | ------------ | -------- | ------------- | --------- |
| Content (.cidx)    | 350.1 MB     | 223.7 MB     | 1.6×     | +2.28s        | 1.19s     |
| Definition (.didx) | 328.0 MB     | 103.1 MB     | 3.2×     | +1.46s        | 1.28s     |
| **Total**          | **678.1 MB** | **326.8 MB** | **2.1×** | **+3.74s**    | —         |

### Compression analysis

- **Definition index** compresses well (3.2×) — AST data has repetitive string paths, enum variants, and structural patterns
- **Content index** compresses less (1.6×) — inverted index posting lists (file IDs, positions, line numbers) are already compact integers with less redundancy
- **Save overhead** is acceptable: +3.7s total on top of 25-27s build times (~14% overhead)
- **Load time** is comparable to pre-compression (~1.2s vs ~1.0s) — LZ4 decompresses at ~2-4 GB/s

## Breaking Changes

None. Legacy uncompressed index files are auto-detected on load and work without errors. New files are always saved compressed.

## Design Decisions

1. **LZ4 frame format** (not block) — self-describing, handles streaming, standard framing
2. **Magic bytes `LZ4S`** — enables backward compatibility detection without file extension changes
3. **Centralized helpers** — single `save_compressed`/`load_compressed` pair used by all 3 index types
4. **`lz4_flex` crate** — pure Rust, no C/C++ build dependency, well-maintained
5. **Streaming serialization** — `bincode::serialize_into` writes directly to `FrameEncoder`, avoiding double-buffering


---

## PR #16 -- Fix substring AND-mode false positives, eliminate 100MB allocation wa…

- **Link:** [https://github.com/pustynsky/search-index/pull/16](https://github.com/pustynsky/search-index/pull/16)
- **Status:** merged
- **Created:** 2026-02-18T18:56:29Z

### Description

## Summary

Fixes 3 P1 issues identified during code review: a correctness bug in substring AND-mode search, a wasteful ~100MB memory allocation during definition reindexing, and a version desync between Cargo.toml and MCP protocol response.

## Changes

### 🐛 Fix: Substring AND-mode search returns false positives

**File:** `src/mcp/handlers/grep.rs`

In substring search, `terms_matched` was incremented per matching **token** instead of per search **term**. When a single search term (e.g., `"service"`) matched multiple tokens via the trigram index (`userservice`, `servicehelper`, `servicemanager`), the counter reached 3 instead of 1. The AND filter (`terms_matched >= term_count`) then incorrectly passed files where only one of two search terms actually matched.

**Fix:** Added `HashMap<u32, HashSet<usize>>` to track distinct matched term indices per file. The AND filter now checks the number of unique matched terms instead of the raw token match count.

### ⚡ Fix: Eliminate ~100MB allocation in `reindex_definitions`

**File:** `src/mcp/handlers/mod.rs`

`bincode::serialize(&new_index)` was called purely to get `.len()` for a cosmetic `sizeMb` response field — allocating the entire serialized index (~100+ MB) into memory.

**Fix:** Replaced with `bincode::serialized_size(&new_index)` which computes the byte count without allocation.

### 🔧 Fix: Version desync (Cargo.toml 0.1.0 vs protocol.rs 0.3.0)

**File:** `src/mcp/protocol.rs`

The MCP `InitializeResult` hardcoded `version: "0.3.0"` while `Cargo.toml` has `version = "0.1.0"`.

**Fix:** Replaced with `env!("CARGO_PKG_VERSION")` to always derive from Cargo.toml.

## Testing

- **2 new unit tests** in `handlers_tests.rs`:
  - `test_substring_and_mode_no_false_positive_from_multi_token_match` — verifies AND-mode rejects files where only 1 of 2 terms matches (via 3 tokens)
  - `test_substring_and_mode_correct_when_both_terms_match` — verifies AND-mode passes when both terms match
- **E2E test plan** updated with scenario T37c
- **All 264 tests passing**, 0 failures

---

## PR #17 -- feat: async MCP server startup — respond to initialize immediately, b…

- **Link:** [https://github.com/pustynsky/search-index/pull/17](https://github.com/pustynsky/search-index/pull/17)
- **Status:** merged
- **Created:** 2026-02-18T19:19:36Z

### Description

## Branch Name

`async-mcp-startup`

## PR Title

feat: async MCP server startup — respond to initialize immediately, build indexes in background

## PR Description

### Problem

When the MCP server starts for the first time (no pre-built indexes on disk), `cmd_serve()` blocks on `build_content_index()` and `build_definition_index()` for 30–300 seconds **before** starting the event loop. Roo/VS Code sends `initialize` immediately after spawning the process, gets no response, times out, and kills the process. Users must pre-build indexes via CLI or restart VS Code.

### Solution

**"Start immediately, build in background"** — the server creates empty indexes, starts the event loop, and responds to `initialize` / `tools/list` instantly. Indexes are loaded from disk synchronously if available (fast path, < 3s) or built in background threads (slow path, 30–300s).

### Architecture

- Empty `ContentIndex` / `DefinitionIndex` wrapped in `Arc<RwLock<T>>` 
- `Arc<AtomicBool>` flags (`content_ready`, `def_ready`) with `Release`/`Acquire` ordering
- `dispatch_tool()` checks readiness before routing to search handlers
- Tools that don't need indexes (`search_help`, `search_info`, `search_find`) work immediately
- `search_reindex` during background build returns "already building" error

### Behavior during index build

| MCP request | Response |
|---|---|
| `initialize` | ✅ Instant |
| `tools/list` | ✅ Full tool list |
| `search_grep` / `search_fast` | ⚠️ "Content index is being built, please retry" |
| `search_definitions` / `search_callers` | ⚠️ "Definition index is being built, please retry" |
| `search_reindex` | ⚠️ "Already being built, please wait" |
| `search_help` / `search_info` / `search_find` | ✅ Works immediately |

### Files changed

| File | Change |
|---|---|
| `src/cli/serve.rs` | Async startup: empty indexes → try disk load → background build if needed |
| `src/mcp/handlers/mod.rs` | Added `content_ready`/`def_ready` to `HandlerContext`; readiness checks in `dispatch_tool()` |
| `src/mcp/server.rs` | Pass `AtomicBool` flags through `run_server()` |
| `src/mcp/handlers/handlers_tests.rs` | 7 new tests for index-building behavior |
| `docs/e2e-test-plan.md` | 9 async startup test scenarios (T-ASYNC-01 – T-ASYNC-09) |
| `docs/architecture.md` | Added async startup design decision |
| `docs/concurrency.md` | Added Phase 2.5 (async startup), updated thread safety table |
| `docs/storage.md` | Updated "Index missing" behavior for `search serve` |
| `docs/tradeoffs.md` | Updated locking rationale |
| `docs/async-startup-design.md` | Status → ✅ Реализовано |
| `README.md` | Updated MCP Server feature + `search serve` documentation |

### Testing

- **271 tests pass** (264 existing + 7 new async startup tests)
- 0 failures, 0 warnings
- Binary reinstalled via `cargo install --path . --force`

---

## PR #18 -- feat: save-on-shutdown preserves incremental watcher index updates

- **Link:** [https://github.com/pustynsky/search-index/pull/18](https://github.com/pustynsky/search-index/pull/18)
- **Status:** merged
- **Created:** 2026-02-18T20:02:42Z

### Description

## Save indexes on graceful shutdown

### Problem

The MCP server's file watcher applies incremental updates to in-memory content and definition indexes, but these updates were **never saved to disk**. Only bulk reindex (triggered by `bulk_threshold`) persisted the index. On server restart, all incremental changes were lost — the server loaded a potentially stale index from disk, and the watcher only detected changes happening *after* startup (not during downtime).

### Solution

Added `save_indexes_on_shutdown()` in `server.rs` — when stdin closes (standard MCP server shutdown triggered by VS Code), both content and definition indexes are saved to disk before the process exits.

### Changes

| File | Description |
|------|-------------|
| `src/mcp/server.rs` | Added `save_indexes_on_shutdown()` — saves content + definition indexes when stdin closes |
| `src/mcp/watcher.rs` | Added unit test `test_watch_index_survives_save_load_roundtrip` — verifies watch-mode fields (`forward`, `path_to_id`) survive serialization roundtrip |
| `e2e-test.ps1` | Added automated T-SHUTDOWN test — starts server with `--watch`, modifies a file, closes stdin, verifies save log in stderr |
| `docs/e2e-test-plan.md` | Added T-SHUTDOWN test specification |

### Risk Assessment

- **Risk: 🟢 Very low** — code runs once at shutdown, after the event loop exits
- If save fails — server is already terminating, no impact
- No runtime performance impact (zero overhead during normal operation)
- Covers ~95% of restart scenarios (VS Code closes stdin on MCP server stop)
- Does NOT cover `kill -9` / crash — would require periodic save (future work)

### Testing

- **272 unit tests pass** (0 failures)
- **24 E2E tests pass** (0 failures), including new T-SHUTDOWN
- Unit test verifies `forward`/`path_to_id` fields survive save/load roundtrip

---

## PR #19 -- feat: phrase search raw content matching for punctuation-heavy queries

- **Link:** [https://github.com/pustynsky/search-index/pull/19](https://github.com/pustynsky/search-index/pull/19)
- **Status:** merged
- **Created:** 2026-02-18T20:38:20Z

### Description

## Phrase search raw content matching

### Problem

`search_grep` with `phrase: true` tokenizes the search phrase before matching, stripping XML/code punctuation (`<`, `/`, `>`, etc.). This causes false positives when searching for literal XML/code patterns like `</Property> </Property>` or `ILogger<string>`.

### Solution

When the phrase contains non-alphanumeric characters (punctuation), use direct case-insensitive substring matching against raw file content instead of the tokenized phrase regex. For alphanumeric-only phrases (e.g., `pub fn`), existing tokenized regex matching is unchanged.

### Changes

| File                                 | Change                                                                                                    |
| ------------------------------------ | --------------------------------------------------------------------------------------------------------- |
| `src/mcp/handlers/grep.rs`           | Core logic: raw substring match when phrase has punctuation (~15 lines in `handle_phrase_search`)         |
| `src/mcp/handlers/handlers_tests.rs` | 3 new unit tests: XML literal match, no-punctuation regex mode, angle brackets                            |
| `docs/e2e-test-plan.md`              | Added T37d test case                                                                                      |
| `e2e-test.ps1`                       | Added T15b `grep-phrase-punct` CLI test                                                                   |
| `README.md`                          | Updated `--phrase` flag description                                                                       |
| `.roo/rules/project-context.md`      | Updated Post-Change Checklist: added e2e-test.ps1 evaluation step, e2e run step, and git workflow section |

### Behavior

| Scenario                            | Before               | After                      |
| ----------------------------------- | -------------------- | -------------------------- |
| `phrase: "pub fn"` (no punctuation) | N results via regex  | Same N results (unchanged) |
| `phrase: "</Property> </Property>"` | Many false positives | Only literal matches       |
| `phrase: "ILogger<string>"`         | Many false positives | Only literal matches       |

### Test results

- **Unit tests:** 275 passed, 0 failed
- **E2E tests:** 25 passed, 0 failed


---

## PR #20 -- feat: memory optimization — eliminate forward index + drop+reload aft…

- **Link:** [https://github.com/pustynsky/search-index/pull/20](https://github.com/pustynsky/search-index/pull/20)
- **Status:** merged
- **Created:** 2026-02-18T21:34:17Z

### Description

# Memory optimization: eliminate forward index + early drop of file data

## Problem

`search.exe` with `--watch` consumed ~4 GB RAM on a large repo (~80K files):

- **Steady-state (after load from disk):** ~2.1 GB
- **During index build (first launch):** ~3.7 GB — 1.6 GB higher due to raw file contents held in memory

## Root Causes

1. **Forward index** (`ContentIndex.forward: HashMap<u32, Vec<String>>`) — cloned every token string for every file, consuming ~1.5 GB RAM in `--watch` mode
2. **File contents not freed during build** — `file_data: Vec<(String, String)>` held all raw file contents (~1.6 GB for 80K files) in memory throughout the entire `build_content_index()` function, even after tokenization was complete
3. **Allocator fragmentation after build** — build-time temporary allocations fragment the heap; freed pages can't be returned to OS, causing ~1.5 GB overhead vs. loading from disk

## Solutions

### Fix 1: Eliminate forward index (~1.5 GB savings in steady-state)

Replaced forward index lookup with brute-force scan of inverted index when removing stale postings. Trade-off: ~50-100ms per file change (vs ~5ms before) — acceptable for watcher debounce.

### Fix 2: Early drop of file data (~200 MB savings during build)

Added `drop(file_data)` after tokenization phase to free raw file contents before merge/trigram build phases.

### Fix 3: Drop+reload after build (~1.5 GB savings during build)

After building and saving the index to disk, drop the fragmented build-time index and reload from disk to get compact, contiguous allocations. Cost: ~1.2s extra startup in background thread (not blocking).

## Changes

### Core implementation

- **`src/index.rs`** — `drop(file_data)` after tokenization to free ~1.6 GB during build
- **`src/lib.rs`** — `forward` field marked as deprecated (kept for serde backward-compat)
- **`src/mcp/watcher.rs`** — `build_watch_index_from()` no longer builds forward index; new `purge_file_from_inverted_index()` for brute-force cleanup; `update_file_in_index()` and `remove_file_from_index()` rewritten
- **`src/cli/serve.rs`** — `forward` always set to `None`; drop+reload pattern after build for both content and definition indexes

### Documentation

- **`docs/architecture.md`** — Updated incremental update path description
- **`docs/concurrency.md`** — Added memory optimization note
- **`docs/e2e-test-plan.md`** — Added T50b test scenario
- **`.roo/rules/project-context.md`** — Added environment rules (Windows-only, mandatory testing)

### Tests (6 new unit tests)

- `test_purge_file_from_inverted_index_removes_single_file`
- `test_purge_file_from_inverted_index_nonexistent_file`
- `test_purge_file_from_inverted_index_empty_index`
- `test_build_watch_drops_legacy_forward_index`
- `test_remove_file_without_forward_index`
- `test_update_existing_file_without_forward_index`

## Test Results

- **280 unit tests** — all pass (was 274, added 6)
- **38 lib tests** — all pass
- **24 E2E tests** — all pass

## Expected Impact

| Scenario                               | Before  | After   | Savings |
| -------------------------------------- | ------- | ------- | ------- |
| Steady-state (--watch, load from disk) | ~3.7 GB | ~2.1 GB | ~1.5 GB |
| Peak during build (first launch)       | ~3.7 GB | ~2.1 GB | ~1.6 GB |


---

## PR #21 -- Docs fixes

- **Link:** [https://github.com/pustynsky/search-index/pull/21](https://github.com/pustynsky/search-index/pull/21)
- **Status:** merged
- **Created:** 2026-02-18T22:18:35Z

### Description

*No description provided.*

---

