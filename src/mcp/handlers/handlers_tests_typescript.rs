//! TypeScript-specific handler tests — definitions, callers, includeBody, containsLine.
//! Split from handlers_tests.rs for maintainability. Mirrors handlers_tests_csharp.rs patterns.

use super::*;
use super::handlers_test_utils::cleanup_tmp;
use crate::index::build_trigram_index;
use crate::Posting;
use crate::TrigramIndex;
use crate::definitions::DefinitionEntry;
use crate::definitions::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

// ─── Helpers ─────────────────────────────────────────────────────────

/// Helper: create a context with both content + definition indexes (TypeScript classes/methods/functions/etc).
fn make_ts_ctx_with_defs() -> HandlerContext {
    // Content index: tokens -> files+lines (all lowercase)
    let mut content_idx = HashMap::new();
    content_idx.insert("getuser".to_string(), vec![
        Posting { file_id: 0, lines: vec![15] },
        Posting { file_id: 1, lines: vec![20] },
    ]);
    content_idx.insert("userservice".to_string(), vec![
        Posting { file_id: 0, lines: vec![1, 15] },
        Posting { file_id: 1, lines: vec![5] },
    ]);
    content_idx.insert("orderprocessor".to_string(), vec![
        Posting { file_id: 1, lines: vec![1] },
    ]);
    content_idx.insert("handleorder".to_string(), vec![
        Posting { file_id: 1, lines: vec![18] },
    ]);

    let trigram = build_trigram_index(&content_idx);

    let content_index = ContentIndex {
        root: ".".to_string(),
        created_at: 0,
        max_age_secs: 3600,
        files: vec![
            "src/services/UserService.ts".to_string(),
            "src/processors/OrderProcessor.ts".to_string(),
            "src/utils/helpers.ts".to_string(),
        ],
        index: content_idx,
        total_tokens: 300,
        extensions: vec!["ts".to_string()],
        file_token_counts: vec![100, 100, 100],
        trigram,
        trigram_dirty: false,
        forward: None,
        path_to_id: None,
    };

    // Definitions: all TS definition kinds
    let definitions = vec![
        // 0: Class UserService (file 0)
        DefinitionEntry {
            file_id: 0, name: "UserService".to_string(),
            kind: DefinitionKind::Class, line_start: 1, line_end: 50,
            parent: None, signature: None,
            modifiers: vec!["export".to_string()],
            attributes: vec![],
            base_types: vec!["IUserService".to_string()],
        },
        // 1: Class OrderProcessor (file 1)
        DefinitionEntry {
            file_id: 1, name: "OrderProcessor".to_string(),
            kind: DefinitionKind::Class, line_start: 1, line_end: 60,
            parent: None, signature: None,
            modifiers: vec!["export".to_string(), "abstract".to_string()],
            attributes: vec![],
            base_types: vec![],
        },
        // 2: Interface IUserService (file 0)
        DefinitionEntry {
            file_id: 0, name: "IUserService".to_string(),
            kind: DefinitionKind::Interface, line_start: 55, line_end: 70,
            parent: None, signature: None,
            modifiers: vec![],
            attributes: vec![],
            base_types: vec![],
        },
        // 3: Method getUser (file 0, parent: UserService)
        DefinitionEntry {
            file_id: 0, name: "getUser".to_string(),
            kind: DefinitionKind::Method, line_start: 10, line_end: 25,
            parent: Some("UserService".to_string()), signature: None,
            modifiers: vec!["async".to_string()],
            attributes: vec![],
            base_types: vec![],
        },
        // 4: Method handleOrder (file 1, parent: OrderProcessor)
        DefinitionEntry {
            file_id: 1, name: "handleOrder".to_string(),
            kind: DefinitionKind::Method, line_start: 15, line_end: 30,
            parent: Some("OrderProcessor".to_string()), signature: None,
            modifiers: vec![],
            attributes: vec![],
            base_types: vec![],
        },
        // 5: Constructor (file 0, parent: UserService)
        DefinitionEntry {
            file_id: 0, name: "constructor".to_string(),
            kind: DefinitionKind::Constructor, line_start: 5, line_end: 9,
            parent: Some("UserService".to_string()), signature: None,
            modifiers: vec![],
            attributes: vec![],
            base_types: vec![],
        },
        // 6: Function createLogger (file 2)
        DefinitionEntry {
            file_id: 2, name: "createLogger".to_string(),
            kind: DefinitionKind::Function, line_start: 1, line_end: 10,
            parent: None, signature: None,
            modifiers: vec!["export".to_string()],
            attributes: vec![],
            base_types: vec![],
        },
        // 7: Enum UserStatus (file 2)
        DefinitionEntry {
            file_id: 2, name: "UserStatus".to_string(),
            kind: DefinitionKind::Enum, line_start: 12, line_end: 18,
            parent: None, signature: None,
            modifiers: vec![],
            attributes: vec![],
            base_types: vec![],
        },
        // 8: EnumMember Active (file 2, parent: UserStatus)
        DefinitionEntry {
            file_id: 2, name: "Active".to_string(),
            kind: DefinitionKind::EnumMember, line_start: 13, line_end: 13,
            parent: Some("UserStatus".to_string()), signature: None,
            modifiers: vec![],
            attributes: vec![],
            base_types: vec![],
        },
        // 9: EnumMember Inactive (file 2, parent: UserStatus)
        DefinitionEntry {
            file_id: 2, name: "Inactive".to_string(),
            kind: DefinitionKind::EnumMember, line_start: 14, line_end: 14,
            parent: Some("UserStatus".to_string()), signature: None,
            modifiers: vec![],
            attributes: vec![],
            base_types: vec![],
        },
        // 10: TypeAlias UserId (file 2)
        DefinitionEntry {
            file_id: 2, name: "UserId".to_string(),
            kind: DefinitionKind::TypeAlias, line_start: 20, line_end: 20,
            parent: None,
            signature: Some("type UserId = string | number".to_string()),
            modifiers: vec![],
            attributes: vec![],
            base_types: vec![],
        },
        // 11: Variable DEFAULT_TIMEOUT (file 2)
        DefinitionEntry {
            file_id: 2, name: "DEFAULT_TIMEOUT".to_string(),
            kind: DefinitionKind::Variable, line_start: 22, line_end: 22,
            parent: None, signature: None,
            modifiers: vec!["export".to_string(), "const".to_string()],
            attributes: vec![],
            base_types: vec![],
        },
        // 12: Field name (file 0, parent: UserService)
        DefinitionEntry {
            file_id: 0, name: "name".to_string(),
            kind: DefinitionKind::Field, line_start: 3, line_end: 3,
            parent: Some("UserService".to_string()), signature: None,
            modifiers: vec!["private".to_string()],
            attributes: vec![],
            base_types: vec![],
        },
        // 13: Property id (file 0, parent: IUserService)
        DefinitionEntry {
            file_id: 0, name: "id".to_string(),
            kind: DefinitionKind::Property, line_start: 57, line_end: 57,
            parent: Some("IUserService".to_string()), signature: None,
            modifiers: vec![],
            attributes: vec![],
            base_types: vec![],
        },
    ];

    let mut name_index: HashMap<String, Vec<u32>> = HashMap::new();
    let mut kind_index: HashMap<DefinitionKind, Vec<u32>> = HashMap::new();
    let mut file_index: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut path_to_id: HashMap<PathBuf, u32> = HashMap::new();
    let mut base_type_index: HashMap<String, Vec<u32>> = HashMap::new();

    for (i, def) in definitions.iter().enumerate() {
        let idx = i as u32;
        name_index.entry(def.name.to_lowercase()).or_default().push(idx);
        kind_index.entry(def.kind.clone()).or_default().push(idx);
        file_index.entry(def.file_id).or_default().push(idx);
        for bt in &def.base_types {
            base_type_index.entry(bt.to_lowercase()).or_default().push(idx);
        }
    }

    path_to_id.insert(PathBuf::from("src/services/UserService.ts"), 0);
    path_to_id.insert(PathBuf::from("src/processors/OrderProcessor.ts"), 1);
    path_to_id.insert(PathBuf::from("src/utils/helpers.ts"), 2);

    // method_calls for "down" direction: handleOrder calls getUser
    let mut method_calls: HashMap<u32, Vec<CallSite>> = HashMap::new();
    method_calls.insert(4, vec![CallSite {
        method_name: "getUser".to_string(),
        receiver_type: Some("UserService".to_string()),
        line: 20,
    }]);

    let def_index = DefinitionIndex {
        root: ".".to_string(),
        created_at: 0,
        extensions: vec!["ts".to_string()],
        files: vec![
            "src/services/UserService.ts".to_string(),
            "src/processors/OrderProcessor.ts".to_string(),
            "src/utils/helpers.ts".to_string(),
        ],
        definitions,
        name_index,
        kind_index,
        attribute_index: HashMap::new(),
        base_type_index,
        file_index,
        path_to_id,
        method_calls,
        parse_errors: 0,
        lossy_file_count: 0,
        empty_file_ids: Vec::new(),
    };

    HandlerContext {
        index: Arc::new(RwLock::new(content_index)),
        def_index: Some(Arc::new(RwLock::new(def_index))),
        server_dir: ".".to_string(),
        server_ext: "ts".to_string(),
        metrics: false,
        index_base: PathBuf::from("."),
        max_response_bytes: crate::mcp::handlers::utils::DEFAULT_MAX_RESPONSE_BYTES,
        content_ready: Arc::new(AtomicBool::new(true)),
        def_ready: Arc::new(AtomicBool::new(true)),
    }
}

/// Helper: create a context with real temp .ts files and a definition index.
fn make_ts_ctx_with_real_files() -> (HandlerContext, std::path::PathBuf) {
    use std::io::Write;
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let tmp_dir = std::env::temp_dir().join(format!("search_test_ts_{}_{}", std::process::id(), id));
    let _ = std::fs::create_dir_all(&tmp_dir);

    // file 0: UserService.ts — 15 lines
    let file0_path = tmp_dir.join("UserService.ts");
    {
        let mut f = std::fs::File::create(&file0_path).unwrap();
        writeln!(f, "export class UserService {{").unwrap();         // line 1
        writeln!(f, "  private name: string;").unwrap();             // line 2
        writeln!(f, "  constructor() {{").unwrap();                   // line 3
        writeln!(f, "    this.name = '';").unwrap();                  // line 4
        writeln!(f, "  }}").unwrap();                                 // line 5
        writeln!(f, "  async getUser(id: number) {{").unwrap();      // line 6
        writeln!(f, "    // fetch user").unwrap();                    // line 7
        writeln!(f, "    const user = await fetch(id);").unwrap();   // line 8
        writeln!(f, "    return user;").unwrap();                     // line 9
        writeln!(f, "  }}").unwrap();                                 // line 10
        writeln!(f, "}}").unwrap();                                   // line 11
        writeln!(f, "").unwrap();                                     // line 12
        writeln!(f, "export interface IUserService {{").unwrap();     // line 13
        writeln!(f, "  id: number;").unwrap();                       // line 14
        writeln!(f, "}}").unwrap();                                   // line 15
    }

    // file 1: OrderProcessor.ts — 20 lines
    let file1_path = tmp_dir.join("OrderProcessor.ts");
    {
        let mut f = std::fs::File::create(&file1_path).unwrap();
        for i in 1..=20 { writeln!(f, "// order processor line {}", i).unwrap(); }
    }

    let file0_str = file0_path.to_string_lossy().to_string();
    let file1_str = file1_path.to_string_lossy().to_string();

    let definitions = vec![
        DefinitionEntry {
            file_id: 0, name: "UserService".to_string(),
            kind: DefinitionKind::Class, line_start: 1, line_end: 11,
            parent: None, signature: None, modifiers: vec![], attributes: vec![], base_types: vec![],
        },
        DefinitionEntry {
            file_id: 0, name: "getUser".to_string(),
            kind: DefinitionKind::Method, line_start: 6, line_end: 10,
            parent: Some("UserService".to_string()), signature: None,
            modifiers: vec![], attributes: vec![], base_types: vec![],
        },
        DefinitionEntry {
            file_id: 1, name: "OrderProcessor".to_string(),
            kind: DefinitionKind::Class, line_start: 1, line_end: 20,
            parent: None, signature: None, modifiers: vec![], attributes: vec![], base_types: vec![],
        },
        DefinitionEntry {
            file_id: 1, name: "handleOrder".to_string(),
            kind: DefinitionKind::Method, line_start: 5, line_end: 19,
            parent: Some("OrderProcessor".to_string()), signature: None,
            modifiers: vec![], attributes: vec![], base_types: vec![],
        },
    ];

    let mut name_index: HashMap<String, Vec<u32>> = HashMap::new();
    let mut kind_index: HashMap<DefinitionKind, Vec<u32>> = HashMap::new();
    let mut file_index: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut path_to_id: HashMap<PathBuf, u32> = HashMap::new();
    for (i, def) in definitions.iter().enumerate() {
        let idx = i as u32;
        name_index.entry(def.name.to_lowercase()).or_default().push(idx);
        kind_index.entry(def.kind.clone()).or_default().push(idx);
        file_index.entry(def.file_id).or_default().push(idx);
    }
    path_to_id.insert(file0_path, 0);
    path_to_id.insert(file1_path, 1);

    let def_index = DefinitionIndex {
        root: tmp_dir.to_string_lossy().to_string(), created_at: 0,
        extensions: vec!["ts".to_string()],
        files: vec![file0_str.clone(), file1_str.clone()],
        definitions, name_index, kind_index,
        attribute_index: HashMap::new(), base_type_index: HashMap::new(),
        file_index, path_to_id, method_calls: HashMap::new(),
        parse_errors: 0, lossy_file_count: 0, empty_file_ids: Vec::new(),
    };

    let content_index = ContentIndex {
        root: tmp_dir.to_string_lossy().to_string(), created_at: 0, max_age_secs: 3600,
        files: vec![file0_str, file1_str],
        index: HashMap::new(), total_tokens: 0,
        extensions: vec!["ts".to_string()],
        file_token_counts: vec![0, 0],
        trigram: TrigramIndex::default(), trigram_dirty: false, forward: None, path_to_id: None,
    };

    let ctx = HandlerContext {
        index: Arc::new(RwLock::new(content_index)),
        def_index: Some(Arc::new(RwLock::new(def_index))),
        server_dir: tmp_dir.to_string_lossy().to_string(),
        server_ext: "ts".to_string(),
        metrics: false,
        index_base: PathBuf::from("."),
        max_response_bytes: crate::mcp::handlers::utils::DEFAULT_MAX_RESPONSE_BYTES,
        content_ready: Arc::new(AtomicBool::new(true)),
        def_ready: Arc::new(AtomicBool::new(true)),
    };
    (ctx, tmp_dir)
}

// ─── Part 2: search_definitions tests (one test per kind) ────────────

#[test]
fn test_ts_search_definitions_finds_class() {
    let ctx = make_ts_ctx_with_defs();
    let result = dispatch_tool(&ctx, "search_definitions", &json!({
        "name": "UserService",
        "kind": "class"
    }));
    assert!(!result.is_error, "search_definitions should not error: {}", result.content[0].text);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let defs = output["definitions"].as_array().unwrap();
    assert_eq!(defs.len(), 1, "Expected exactly 1 class named UserService, got {}", defs.len());
    assert_eq!(defs[0]["name"], "UserService");
    assert_eq!(defs[0]["kind"], "class");
    assert!(defs[0]["file"].as_str().unwrap().contains("UserService.ts"));
}

#[test]
fn test_ts_search_definitions_finds_interface() {
    let ctx = make_ts_ctx_with_defs();
    let result = dispatch_tool(&ctx, "search_definitions", &json!({
        "name": "IUserService",
        "kind": "interface"
    }));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let defs = output["definitions"].as_array().unwrap();
    assert_eq!(defs.len(), 1, "Expected exactly 1 interface named IUserService, got {}", defs.len());
    assert_eq!(defs[0]["name"], "IUserService");
    assert_eq!(defs[0]["kind"], "interface");
}

#[test]
fn test_ts_search_definitions_finds_method() {
    let ctx = make_ts_ctx_with_defs();
    let result = dispatch_tool(&ctx, "search_definitions", &json!({
        "name": "getUser",
        "kind": "method"
    }));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let defs = output["definitions"].as_array().unwrap();
    assert_eq!(defs.len(), 1, "Expected exactly 1 method named getUser, got {}", defs.len());
    assert_eq!(defs[0]["name"], "getUser");
    assert_eq!(defs[0]["kind"], "method");
    assert_eq!(defs[0]["parent"], "UserService");
}

#[test]
fn test_ts_search_definitions_finds_function() {
    let ctx = make_ts_ctx_with_defs();
    let result = dispatch_tool(&ctx, "search_definitions", &json!({
        "kind": "function"
    }));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let defs = output["definitions"].as_array().unwrap();
    assert_eq!(defs.len(), 1, "Expected exactly 1 function, got {}", defs.len());
    assert_eq!(defs[0]["name"], "createLogger");
    assert_eq!(defs[0]["kind"], "function");
    assert!(defs[0]["file"].as_str().unwrap().contains("helpers.ts"));
}

#[test]
fn test_ts_search_definitions_finds_enum() {
    let ctx = make_ts_ctx_with_defs();
    let result = dispatch_tool(&ctx, "search_definitions", &json!({
        "kind": "enum"
    }));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let defs = output["definitions"].as_array().unwrap();
    assert_eq!(defs.len(), 1, "Expected exactly 1 enum, got {}", defs.len());
    assert_eq!(defs[0]["name"], "UserStatus");
    assert_eq!(defs[0]["kind"], "enum");
}

#[test]
fn test_ts_search_definitions_finds_enum_member() {
    let ctx = make_ts_ctx_with_defs();
    let result = dispatch_tool(&ctx, "search_definitions", &json!({
        "kind": "enumMember"
    }));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let defs = output["definitions"].as_array().unwrap();
    assert_eq!(defs.len(), 2, "Expected exactly 2 enum members (Active, Inactive), got {}", defs.len());
    let names: Vec<&str> = defs.iter().filter_map(|d| d["name"].as_str()).collect();
    assert!(names.contains(&"Active"), "Should contain Active enum member");
    assert!(names.contains(&"Inactive"), "Should contain Inactive enum member");
    for def in defs {
        assert_eq!(def["kind"], "enumMember");
        assert_eq!(def["parent"], "UserStatus");
    }
}

#[test]
fn test_ts_search_definitions_finds_type_alias() {
    let ctx = make_ts_ctx_with_defs();
    let result = dispatch_tool(&ctx, "search_definitions", &json!({
        "kind": "typeAlias"
    }));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let defs = output["definitions"].as_array().unwrap();
    assert_eq!(defs.len(), 1, "Expected exactly 1 typeAlias, got {}", defs.len());
    assert_eq!(defs[0]["name"], "UserId");
    assert_eq!(defs[0]["kind"], "typeAlias");
}

#[test]
fn test_ts_search_definitions_finds_variable() {
    let ctx = make_ts_ctx_with_defs();
    let result = dispatch_tool(&ctx, "search_definitions", &json!({
        "kind": "variable"
    }));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let defs = output["definitions"].as_array().unwrap();
    assert_eq!(defs.len(), 1, "Expected exactly 1 variable, got {}", defs.len());
    assert_eq!(defs[0]["name"], "DEFAULT_TIMEOUT");
    assert_eq!(defs[0]["kind"], "variable");
}

#[test]
fn test_ts_search_definitions_finds_field() {
    let ctx = make_ts_ctx_with_defs();
    let result = dispatch_tool(&ctx, "search_definitions", &json!({
        "kind": "field"
    }));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let defs = output["definitions"].as_array().unwrap();
    assert_eq!(defs.len(), 1, "Expected exactly 1 field, got {}", defs.len());
    assert_eq!(defs[0]["name"], "name");
    assert_eq!(defs[0]["kind"], "field");
    assert_eq!(defs[0]["parent"], "UserService");
}

#[test]
fn test_ts_search_definitions_finds_constructor() {
    let ctx = make_ts_ctx_with_defs();
    let result = dispatch_tool(&ctx, "search_definitions", &json!({
        "kind": "constructor"
    }));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let defs = output["definitions"].as_array().unwrap();
    assert_eq!(defs.len(), 1, "Expected exactly 1 constructor, got {}", defs.len());
    assert_eq!(defs[0]["name"], "constructor");
    assert_eq!(defs[0]["kind"], "constructor");
    assert_eq!(defs[0]["parent"], "UserService");
}

// ─── Part 3: baseType filter tests ───────────────────────────────────

#[test]
fn test_ts_search_definitions_base_type_implements() {
    let ctx = make_ts_ctx_with_defs();
    let result = dispatch_tool(&ctx, "search_definitions", &json!({
        "baseType": "IUserService"
    }));
    assert!(!result.is_error, "baseType filter should not error: {}", result.content[0].text);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let defs = output["definitions"].as_array().unwrap();
    assert_eq!(defs.len(), 1, "Expected exactly 1 definition implementing IUserService, got {}", defs.len());
    assert_eq!(defs[0]["name"], "UserService");
    assert_eq!(defs[0]["kind"], "class");
}

#[test]
fn test_ts_search_definitions_base_type_abstract() {
    let ctx = make_ts_ctx_with_defs();
    // OrderProcessor has modifiers ["export", "abstract"] — search by name to verify modifiers
    let result = dispatch_tool(&ctx, "search_definitions", &json!({
        "name": "OrderProcessor",
        "kind": "class"
    }));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let defs = output["definitions"].as_array().unwrap();
    assert_eq!(defs.len(), 1, "Expected exactly 1 class named OrderProcessor, got {}", defs.len());
    assert_eq!(defs[0]["name"], "OrderProcessor");
    // Verify modifiers include "abstract"
    let modifiers = defs[0]["modifiers"].as_array().unwrap();
    let mod_strs: Vec<&str> = modifiers.iter().filter_map(|m| m.as_str()).collect();
    assert!(mod_strs.contains(&"abstract"),
        "OrderProcessor should have abstract modifier, got: {:?}", mod_strs);
}

// ─── Part 4: containsLine and includeBody tests ─────────────────────

#[test]
fn test_ts_contains_line_finds_method() {
    let (ctx, tmp) = make_ts_ctx_with_real_files();
    let result = dispatch_tool(&ctx, "search_definitions", &json!({
        "file": "UserService",
        "containsLine": 8
    }));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let defs = output["containingDefinitions"].as_array().unwrap();
    assert!(!defs.is_empty(), "Should find containing definitions for line 8");
    let method = defs.iter().find(|d| d["kind"] == "method").unwrap();
    assert_eq!(method["name"], "getUser");
    assert_eq!(method["parent"], "UserService");
    cleanup_tmp(&tmp);
}

#[test]
fn test_ts_search_definitions_include_body() {
    let (ctx, tmp) = make_ts_ctx_with_real_files();
    let result = dispatch_tool(&ctx, "search_definitions", &json!({
        "name": "getUser",
        "includeBody": true
    }));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let defs = output["definitions"].as_array().unwrap();
    assert_eq!(defs.len(), 1);
    let body = defs[0]["body"].as_array().unwrap();
    assert!(body.len() > 0, "Body should have content lines");
    assert_eq!(defs[0]["bodyStartLine"], 6);
    cleanup_tmp(&tmp);
}

// ─── Part 5: search_callers tests ────────────────────────────────────

#[test]
fn test_ts_search_callers_up_finds_caller() {
    let ctx = make_ts_ctx_with_defs();
    let result = dispatch_tool(&ctx, "search_callers", &json!({
        "method": "getUser",
        "class": "UserService",
        "depth": 1
    }));
    assert!(!result.is_error, "search_callers should not error: {}", result.content[0].text);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let tree = output["callTree"].as_array().unwrap();
    assert!(!tree.is_empty(), "Call tree should not be empty — handleOrder calls getUser");
    // Verify the caller is handleOrder in OrderProcessor
    let caller_methods: Vec<&str> = tree.iter().filter_map(|n| n["method"].as_str()).collect();
    assert!(caller_methods.contains(&"handleOrder"),
        "Should find handleOrder as caller, got: {:?}", caller_methods);
}

#[test]
fn test_ts_search_callers_down_finds_callees() {
    let ctx = make_ts_ctx_with_defs();
    let result = dispatch_tool(&ctx, "search_callers", &json!({
        "method": "handleOrder",
        "class": "OrderProcessor",
        "direction": "down",
        "depth": 1
    }));
    assert!(!result.is_error, "search_callers down should not error: {}", result.content[0].text);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let tree = output["callTree"].as_array().unwrap();
    assert!(!tree.is_empty(), "Call tree should not be empty — handleOrder calls getUser");
    let callee_methods: Vec<&str> = tree.iter().filter_map(|n| n["method"].as_str()).collect();
    assert!(callee_methods.contains(&"getUser"),
        "Should find getUser as callee of handleOrder, got: {:?}", callee_methods);
}

#[test]
fn test_ts_search_callers_nonexistent_method() {
    let ctx = make_ts_ctx_with_defs();
    let result = dispatch_tool(&ctx, "search_callers", &json!({
        "method": "nonExistentMethodXYZ"
    }));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let tree = output["callTree"].as_array().unwrap();
    assert!(tree.is_empty(), "Call tree should be empty for nonexistent method");
}

// ─── Part 6: Combined filters ────────────────────────────────────────

#[test]
fn test_ts_search_definitions_combined_name_parent_kind() {
    let ctx = make_ts_ctx_with_defs();
    let result = dispatch_tool(&ctx, "search_definitions", &json!({
        "name": "getUser",
        "parent": "UserService",
        "kind": "method"
    }));
    assert!(!result.is_error, "Combined filter should not error: {}", result.content[0].text);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let defs = output["definitions"].as_array().unwrap();
    assert_eq!(defs.len(), 1,
        "Expected exactly 1 result for name+parent+kind filter, got {}: {:?}",
        defs.len(), defs);
    assert_eq!(defs[0]["name"], "getUser");
    assert_eq!(defs[0]["parent"], "UserService");
    assert_eq!(defs[0]["kind"], "method");

    // Verify: same name+kind but different parent should NOT match
    let result2 = dispatch_tool(&ctx, "search_definitions", &json!({
        "name": "getUser",
        "parent": "NonExistentClass",
        "kind": "method"
    }));
    assert!(!result2.is_error);
    let output2: Value = serde_json::from_str(&result2.content[0].text).unwrap();
    let defs2 = output2["definitions"].as_array().unwrap();
    assert_eq!(defs2.len(), 0,
        "Non-matching parent should return 0 results, got {}", defs2.len());
}

#[test]
fn test_ts_search_definitions_name_regex() {
    let ctx = make_ts_ctx_with_defs();
    let result = dispatch_tool(&ctx, "search_definitions", &json!({
        "name": "User.*",
        "regex": true
    }));
    assert!(!result.is_error, "Regex search should not error: {}", result.content[0].text);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let defs = output["definitions"].as_array().unwrap();

    // Should match: UserService, UserStatus, UserId, IUserService (regex is case-insensitive substring)
    assert!(defs.len() >= 3,
        "Regex 'User.*' should match at least UserService, UserStatus, UserId. Got {}: {:?}",
        defs.len(), defs.iter().map(|d| d["name"].as_str()).collect::<Vec<_>>());

    let names: Vec<&str> = defs.iter().filter_map(|d| d["name"].as_str()).collect();
    assert!(names.contains(&"UserService"), "Should contain UserService");
    assert!(names.contains(&"UserStatus"), "Should contain UserStatus");
    assert!(names.contains(&"UserId"), "Should contain UserId");

    // All returned definitions should contain "user" (case-insensitive) in their name
    for def in defs {
        let name = def["name"].as_str().unwrap();
        assert!(name.to_lowercase().contains("user"),
            "Definition '{}' should match regex 'User.*'", name);
    }
}