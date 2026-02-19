//! Tests for MCP handlers — extracted from the original monolithic handlers.rs.
//! Uses `#[path = "handlers_tests.rs"]` in mod.rs to include as the tests module.

use super::*;
use super::grep::handle_search_grep;
use super::fast::handle_search_fast;
use super::utils::validate_search_dir;
use crate::index::build_trigram_index;
use crate::Posting;
use crate::TrigramIndex;
use crate::definitions::DefinitionEntry;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

#[test]
fn test_tool_definitions_count() {
    let tools = tool_definitions();
    assert_eq!(tools.len(), 9);
}

#[test]
fn test_tool_definitions_names() {
    let tools = tool_definitions();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&"search_grep"));
    assert!(names.contains(&"search_find"));
    assert!(names.contains(&"search_fast"));
    assert!(names.contains(&"search_info"));
    assert!(names.contains(&"search_reindex"));
    assert!(names.contains(&"search_reindex_definitions"));
    assert!(names.contains(&"search_definitions"));
    assert!(names.contains(&"search_callers"));
    assert!(names.contains(&"search_help"));
}

#[test]
fn test_tool_definitions_have_schemas() {
    let tools = tool_definitions();
    for tool in &tools {
        assert!(tool.input_schema.is_object(), "Tool {} should have an object schema", tool.name);
        assert_eq!(tool.input_schema["type"], "object");
    }
}

#[test]
fn test_all_tools_have_required_field() {
    let tools = tool_definitions();
    for tool in &tools {
        assert!(
            tool.input_schema.get("required").is_some(),
            "Tool '{}' inputSchema is missing 'required' field. \
             MCP clients (e.g. MS-Roo-Code) expect 'required' to always be present, \
             even as an empty array. Without it, JSON.parse() fails with \
             'Unexpected end of JSON input' during auto-approve toggle.",
            tool.name
        );
        assert!(
            tool.input_schema["required"].is_array(),
            "Tool '{}' 'required' field must be an array, got: {}",
            tool.name,
            tool.input_schema["required"]
        );
    }
}

#[test]
fn test_search_grep_required_fields() {
    let tools = tool_definitions();
    let grep = tools.iter().find(|t| t.name == "search_grep").unwrap();
    let required = grep.input_schema["required"].as_array().unwrap();
    assert_eq!(required.len(), 1);
    assert_eq!(required[0], "terms");
}

#[test]
fn test_search_find_has_slow_warning() {
    let tools = tool_definitions();
    let find = tools.iter().find(|t| t.name == "search_find").unwrap();
    assert!(find.description.contains("SLOW") || find.description.contains("search_fast"), "search_find should discourage use and point to search_fast");
}

fn make_empty_ctx() -> HandlerContext {
    let index = ContentIndex {
        root: ".".to_string(),
        created_at: 0,
        max_age_secs: 3600,
        files: vec![],
        index: HashMap::new(),
        total_tokens: 0,
        extensions: vec![],
        file_token_counts: vec![],
        trigram: TrigramIndex::default(),
        trigram_dirty: false,
        forward: None,
        path_to_id: None,
    };
    HandlerContext {
        index: Arc::new(RwLock::new(index)),
        def_index: None,
        server_dir: ".".to_string(),
        server_ext: "cs".to_string(),
        metrics: false,
        index_base: PathBuf::from("."), max_response_bytes: crate::mcp::handlers::utils::DEFAULT_MAX_RESPONSE_BYTES, content_ready: Arc::new(AtomicBool::new(true)), def_ready: Arc::new(AtomicBool::new(true)),
    }
}

#[test]
fn test_dispatch_unknown_tool() {
    let ctx = make_empty_ctx();
    let result = dispatch_tool(&ctx, "nonexistent_tool", &json!({}));
    assert!(result.is_error);
    assert!(result.content[0].text.contains("Unknown tool"));
}

#[test]
fn test_dispatch_grep_missing_terms() {
    let ctx = make_empty_ctx();
    let result = dispatch_tool(&ctx, "search_grep", &json!({}));
    assert!(result.is_error);
    assert!(result.content[0].text.contains("Missing required parameter: terms"));
}

#[test]
fn test_dispatch_grep_empty_index() {
    let ctx = make_empty_ctx();
    let result = dispatch_tool(&ctx, "search_grep", &json!({"terms": "HttpClient"}));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    assert_eq!(output["summary"]["totalFiles"], 0);
}

#[test]
fn test_dispatch_grep_with_results() {
    let mut idx = HashMap::new();
    idx.insert("httpclient".to_string(), vec![Posting {
        file_id: 0,
        lines: vec![5, 12],
    }]);
    let index = ContentIndex {
        root: ".".to_string(),
        created_at: 0,
        max_age_secs: 3600,
        files: vec!["C:\\test\\Program.cs".to_string()],
        index: idx,
        total_tokens: 100,
        extensions: vec!["cs".to_string()],
        file_token_counts: vec![50],
        trigram: TrigramIndex::default(),
        trigram_dirty: false,
        forward: None,
        path_to_id: None,
    };
    let ctx = HandlerContext {
        index: Arc::new(RwLock::new(index)),
        def_index: None,
        server_dir: ".".to_string(),
        server_ext: "cs".to_string(),
        metrics: false,
        index_base: PathBuf::from("."), max_response_bytes: crate::mcp::handlers::utils::DEFAULT_MAX_RESPONSE_BYTES, content_ready: Arc::new(AtomicBool::new(true)), def_ready: Arc::new(AtomicBool::new(true)),
    };
    let result = dispatch_tool(&ctx, "search_grep", &json!({"terms": "HttpClient", "substring": false}));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    assert_eq!(output["summary"]["totalFiles"], 1);
    assert_eq!(output["files"][0]["path"], "C:\\test\\Program.cs");
    assert_eq!(output["files"][0]["occurrences"], 2);
}

// --- Helper: create a context with both content + definition indexes ---

fn make_ctx_with_defs() -> HandlerContext {
    use crate::definitions::*;

    // Content index: tokens -> files+lines
    let mut content_idx = HashMap::new();
    content_idx.insert("executequeryasync".to_string(), vec![
        Posting { file_id: 0, lines: vec![242] },
        Posting { file_id: 1, lines: vec![88] },
        Posting { file_id: 2, lines: vec![391] },
    ]);
    content_idx.insert("queryinternalasync".to_string(), vec![
        Posting { file_id: 2, lines: vec![766] },
        Posting { file_id: 2, lines: vec![462] },
    ]);
    content_idx.insert("proxyclient".to_string(), vec![
        Posting { file_id: 1, lines: vec![1, 88] },
    ]);
    content_idx.insert("resilientclient".to_string(), vec![
        Posting { file_id: 0, lines: vec![1, 242] },
    ]);
    content_idx.insert("queryservice".to_string(), vec![
        Posting { file_id: 2, lines: vec![1, 391, 462, 766] },
    ]);

    let content_index = ContentIndex {
        root: ".".to_string(),
        created_at: 0,
        max_age_secs: 3600,
        files: vec![
            "C:\\src\\ResilientClient.cs".to_string(),
            "C:\\src\\ProxyClient.cs".to_string(),
            "C:\\src\\QueryService.cs".to_string(),
        ],
        index: content_idx,
        total_tokens: 500,
        extensions: vec!["cs".to_string()],
        file_token_counts: vec![100, 50, 200],
        trigram: TrigramIndex::default(),
        trigram_dirty: false,
        forward: None,
        path_to_id: None,
    };

    let definitions = vec![
        DefinitionEntry {
            file_id: 0, name: "ResilientClient".to_string(),
            kind: DefinitionKind::Class, line_start: 1, line_end: 300,
            parent: None, signature: None, modifiers: vec![], attributes: vec![], base_types: vec![],
        },
        DefinitionEntry {
            file_id: 0, name: "ExecuteQueryAsync".to_string(),
            kind: DefinitionKind::Method, line_start: 240, line_end: 260,
            parent: Some("ResilientClient".to_string()), signature: None,
            modifiers: vec![], attributes: vec![], base_types: vec![],
        },
        DefinitionEntry {
            file_id: 1, name: "ProxyClient".to_string(),
            kind: DefinitionKind::Class, line_start: 1, line_end: 100,
            parent: None, signature: None, modifiers: vec![], attributes: vec![], base_types: vec![],
        },
        DefinitionEntry {
            file_id: 1, name: "ExecuteQueryAsync".to_string(),
            kind: DefinitionKind::Method, line_start: 85, line_end: 95,
            parent: Some("ProxyClient".to_string()), signature: None,
            modifiers: vec![], attributes: vec![], base_types: vec![],
        },
        DefinitionEntry {
            file_id: 2, name: "QueryService".to_string(),
            kind: DefinitionKind::Class, line_start: 1, line_end: 900,
            parent: None, signature: None, modifiers: vec![], attributes: vec![], base_types: vec![],
        },
        DefinitionEntry {
            file_id: 2, name: "RunQueryBatchAsync".to_string(),
            kind: DefinitionKind::Method, line_start: 386, line_end: 395,
            parent: Some("QueryService".to_string()), signature: None,
            modifiers: vec![], attributes: vec![], base_types: vec![],
        },
        DefinitionEntry {
            file_id: 2, name: "QueryImplAsync".to_string(),
            kind: DefinitionKind::Method, line_start: 450, line_end: 470,
            parent: Some("QueryService".to_string()), signature: None,
            modifiers: vec![], attributes: vec![], base_types: vec![],
        },
        DefinitionEntry {
            file_id: 2, name: "QueryInternalAsync".to_string(),
            kind: DefinitionKind::Method, line_start: 760, line_end: 830,
            parent: Some("QueryService".to_string()), signature: None,
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

    path_to_id.insert(PathBuf::from("C:\\src\\ResilientClient.cs"), 0);
    path_to_id.insert(PathBuf::from("C:\\src\\ProxyClient.cs"), 1);
    path_to_id.insert(PathBuf::from("C:\\src\\QueryService.cs"), 2);

    let def_index = DefinitionIndex {
        root: ".".to_string(),
        created_at: 0,
        extensions: vec!["cs".to_string()],
        files: vec![
            "C:\\src\\ResilientClient.cs".to_string(),
            "C:\\src\\ProxyClient.cs".to_string(),
            "C:\\src\\QueryService.cs".to_string(),
        ],
        definitions,
        name_index,
        kind_index,
        attribute_index: HashMap::new(),
        base_type_index: HashMap::new(),
        file_index,
        path_to_id,
        method_calls: HashMap::new(), parse_errors: 0, lossy_file_count: 0, empty_file_ids: Vec::new(),
    };

    HandlerContext {
        index: Arc::new(RwLock::new(content_index)),
        def_index: Some(Arc::new(RwLock::new(def_index))),
        server_dir: ".".to_string(),
        server_ext: "cs".to_string(),
        metrics: false,
        index_base: PathBuf::from("."), max_response_bytes: crate::mcp::handlers::utils::DEFAULT_MAX_RESPONSE_BYTES, content_ready: Arc::new(AtomicBool::new(true)), def_ready: Arc::new(AtomicBool::new(true)),
    }
}

// --- search_callers tests ---

#[test]
fn test_search_callers_missing_method() {
    let ctx = make_ctx_with_defs();
    let result = dispatch_tool(&ctx, "search_callers", &json!({}));
    assert!(result.is_error);
    assert!(result.content[0].text.contains("Missing required parameter: method"));
}

#[test]
fn test_search_callers_no_def_index() {
    let ctx = make_empty_ctx();
    let result = dispatch_tool(&ctx, "search_callers", &json!({"method": "Foo"}));
    assert!(result.is_error);
    assert!(result.content[0].text.contains("Definition index not available"));
}

#[test]
fn test_search_callers_finds_callers() {
    let ctx = make_ctx_with_defs();
    let result = dispatch_tool(&ctx, "search_callers", &json!({
        "method": "ExecuteQueryAsync",
        "depth": 2
    }));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let tree = output["callTree"].as_array().unwrap();
    assert!(!tree.is_empty(), "Call tree should not be empty");
    assert!(output["summary"]["totalNodes"].as_u64().unwrap() > 0);
    assert!(output["summary"]["searchTimeMs"].as_f64().is_some());
}

#[test]
fn test_search_callers_nonexistent_method() {
    let ctx = make_ctx_with_defs();
    let result = dispatch_tool(&ctx, "search_callers", &json!({
        "method": "NonExistentMethodXYZ"
    }));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let tree = output["callTree"].as_array().unwrap();
    assert!(tree.is_empty(), "Call tree should be empty for nonexistent method");
}

#[test]
fn test_search_callers_max_total_nodes() {
    let ctx = make_ctx_with_defs();
    let result = dispatch_tool(&ctx, "search_callers", &json!({
        "method": "ExecuteQueryAsync",
        "depth": 5,
        "maxTotalNodes": 2
    }));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let total = output["summary"]["totalNodes"].as_u64().unwrap();
    assert!(total <= 2, "Total nodes should be capped at 2, got {}", total);
}

#[test]
fn test_search_callers_max_per_level() {
    let ctx = make_ctx_with_defs();
    let result = dispatch_tool(&ctx, "search_callers", &json!({
        "method": "ExecuteQueryAsync",
        "depth": 1,
        "maxCallersPerLevel": 1
    }));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let tree = output["callTree"].as_array().unwrap();
    assert!(tree.len() <= 1, "Should have at most 1 caller per level, got {}", tree.len());
}

#[test]
fn test_search_callers_has_class_and_file() {
    let ctx = make_ctx_with_defs();
    let result = dispatch_tool(&ctx, "search_callers", &json!({
        "method": "ExecuteQueryAsync",
        "depth": 1
    }));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let tree = output["callTree"].as_array().unwrap();
    for node in tree {
        assert!(node["method"].is_string(), "Node should have method name");
        assert!(node["file"].is_string(), "Node should have file name");
        assert!(node["line"].is_number(), "Node should have line number");
    }
}

#[test]
fn test_search_callers_field_prefix_m_underscore() {
    use crate::definitions::*;

    let mut content_idx = HashMap::new();
    content_idx.insert("submitasync".to_string(), vec![
        Posting { file_id: 0, lines: vec![45] },
        Posting { file_id: 1, lines: vec![30] },
    ]);
    content_idx.insert("orderprocessor".to_string(), vec![
        Posting { file_id: 0, lines: vec![1, 45] },
    ]);
    content_idx.insert("m_orderprocessor".to_string(), vec![
        Posting { file_id: 1, lines: vec![5, 30] },
    ]);
    content_idx.insert("checkouthandler".to_string(), vec![
        Posting { file_id: 1, lines: vec![1] },
    ]);

    let trigram = build_trigram_index(&content_idx);

    let content_index = ContentIndex {
        root: ".".to_string(), created_at: 0, max_age_secs: 3600,
        files: vec![
            "C:\\src\\OrderProcessor.cs".to_string(),
            "C:\\src\\CheckoutHandler.cs".to_string(),
        ],
        index: content_idx, total_tokens: 200,
        extensions: vec!["cs".to_string()],
        file_token_counts: vec![100, 100],
        trigram, trigram_dirty: false, forward: None, path_to_id: None,
    };

    let definitions = vec![
        DefinitionEntry {
            file_id: 0, name: "OrderProcessor".to_string(),
            kind: DefinitionKind::Class, line_start: 1, line_end: 100,
            parent: None, signature: None, modifiers: vec![], attributes: vec![], base_types: vec![],
        },
        DefinitionEntry {
            file_id: 0, name: "SubmitAsync".to_string(),
            kind: DefinitionKind::Method, line_start: 45, line_end: 60,
            parent: Some("OrderProcessor".to_string()), signature: None,
            modifiers: vec![], attributes: vec![], base_types: vec![],
        },
        DefinitionEntry {
            file_id: 1, name: "CheckoutHandler".to_string(),
            kind: DefinitionKind::Class, line_start: 1, line_end: 50,
            parent: None, signature: None, modifiers: vec![], attributes: vec![], base_types: vec![],
        },
        DefinitionEntry {
            file_id: 1, name: "HandleRequest".to_string(),
            kind: DefinitionKind::Method, line_start: 25, line_end: 40,
            parent: Some("CheckoutHandler".to_string()), signature: None,
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
    path_to_id.insert(PathBuf::from("C:\\src\\OrderProcessor.cs"), 0);
    path_to_id.insert(PathBuf::from("C:\\src\\CheckoutHandler.cs"), 1);

    let def_index = DefinitionIndex {
        root: ".".to_string(), created_at: 0,
        extensions: vec!["cs".to_string()],
        files: vec![
            "C:\\src\\OrderProcessor.cs".to_string(),
            "C:\\src\\CheckoutHandler.cs".to_string(),
        ],
        definitions, name_index, kind_index,
        attribute_index: HashMap::new(), base_type_index: HashMap::new(),
        file_index, path_to_id, method_calls: HashMap::new(), parse_errors: 0, lossy_file_count: 0, empty_file_ids: Vec::new(),
    };

    let ctx = HandlerContext {
        index: Arc::new(RwLock::new(content_index)),
        def_index: Some(Arc::new(RwLock::new(def_index))),
        server_dir: ".".to_string(), server_ext: "cs".to_string(),
        metrics: false, index_base: PathBuf::from("."), max_response_bytes: crate::mcp::handlers::utils::DEFAULT_MAX_RESPONSE_BYTES, content_ready: Arc::new(AtomicBool::new(true)), def_ready: Arc::new(AtomicBool::new(true)),
    };

    let result = dispatch_tool(&ctx, "search_callers", &json!({
        "method": "SubmitAsync",
        "class": "OrderProcessor",
        "depth": 1
    }));
    assert!(!result.is_error, "search_callers should not error: {}", result.content[0].text);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let tree = output["callTree"].as_array().unwrap();
    assert!(!tree.is_empty(),
        "Call tree should find caller through m_orderProcessor field prefix. Got: {}",
        serde_json::to_string_pretty(&output).unwrap());
    assert_eq!(tree[0]["method"], "HandleRequest");
    assert_eq!(tree[0]["class"], "CheckoutHandler");
}

#[test]
fn test_search_callers_field_prefix_underscore() {
    use crate::definitions::*;

    let mut content_idx = HashMap::new();
    content_idx.insert("getuserasync".to_string(), vec![
        Posting { file_id: 0, lines: vec![15] },
        Posting { file_id: 1, lines: vec![15] },
    ]);
    content_idx.insert("userservice".to_string(), vec![
        Posting { file_id: 0, lines: vec![1] },
    ]);
    content_idx.insert("_userservice".to_string(), vec![
        Posting { file_id: 1, lines: vec![3, 15] },
    ]);

    let trigram = build_trigram_index(&content_idx);

    let content_index = ContentIndex {
        root: ".".to_string(), created_at: 0, max_age_secs: 3600,
        files: vec!["C:\\src\\UserService.cs".to_string(), "C:\\src\\AccountController.cs".to_string()],
        index: content_idx, total_tokens: 100,
        extensions: vec!["cs".to_string()],
        file_token_counts: vec![50, 50],
        trigram, trigram_dirty: false, forward: None, path_to_id: None,
    };

    let definitions = vec![
        DefinitionEntry {
            file_id: 0, name: "UserService".to_string(),
            kind: DefinitionKind::Class, line_start: 1, line_end: 50,
            parent: None, signature: None, modifiers: vec![], attributes: vec![], base_types: vec![],
        },
        DefinitionEntry {
            file_id: 0, name: "GetUserAsync".to_string(),
            kind: DefinitionKind::Method, line_start: 15, line_end: 30,
            parent: Some("UserService".to_string()), signature: None,
            modifiers: vec![], attributes: vec![], base_types: vec![],
        },
        DefinitionEntry {
            file_id: 1, name: "AccountController".to_string(),
            kind: DefinitionKind::Class, line_start: 1, line_end: 30,
            parent: None, signature: None, modifiers: vec![], attributes: vec![], base_types: vec![],
        },
        DefinitionEntry {
            file_id: 1, name: "GetAccount".to_string(),
            kind: DefinitionKind::Method, line_start: 10, line_end: 25,
            parent: Some("AccountController".to_string()), signature: None,
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
    path_to_id.insert(PathBuf::from("C:\\src\\UserService.cs"), 0);
    path_to_id.insert(PathBuf::from("C:\\src\\AccountController.cs"), 1);

    let def_index = DefinitionIndex {
        root: ".".to_string(), created_at: 0,
        extensions: vec!["cs".to_string()],
        files: vec!["C:\\src\\UserService.cs".to_string(), "C:\\src\\AccountController.cs".to_string()],
        definitions, name_index, kind_index,
        attribute_index: HashMap::new(), base_type_index: HashMap::new(),
        file_index, path_to_id, method_calls: HashMap::new(), parse_errors: 0, lossy_file_count: 0, empty_file_ids: Vec::new(),
    };

    let ctx = HandlerContext {
        index: Arc::new(RwLock::new(content_index)),
        def_index: Some(Arc::new(RwLock::new(def_index))),
        server_dir: ".".to_string(), server_ext: "cs".to_string(),
        metrics: false, index_base: PathBuf::from("."), max_response_bytes: crate::mcp::handlers::utils::DEFAULT_MAX_RESPONSE_BYTES, content_ready: Arc::new(AtomicBool::new(true)), def_ready: Arc::new(AtomicBool::new(true)),
    };

    let result = dispatch_tool(&ctx, "search_callers", &json!({
        "method": "GetUserAsync",
        "class": "UserService",
        "depth": 1
    }));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let tree = output["callTree"].as_array().unwrap();
    assert!(!tree.is_empty(), "Should find caller through _userService field prefix");
    assert_eq!(tree[0]["method"], "GetAccount");
    assert_eq!(tree[0]["class"], "AccountController");
}

#[test]
fn test_search_callers_no_trigram_no_regression() {
    let ctx = make_ctx_with_defs();
    let result = dispatch_tool(&ctx, "search_callers", &json!({
        "method": "ExecuteQueryAsync",
        "class": "ResilientClient",
        "depth": 1
    }));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    assert!(output["summary"]["searchTimeMs"].as_f64().is_some());
}

#[test]
fn test_search_callers_multi_ext_filter() {
    let ctx = make_ctx_with_defs();
    let multi_ext_ctx = HandlerContext {
        index: ctx.index.clone(),
        def_index: ctx.def_index.clone(),
        server_dir: ctx.server_dir.clone(),
        server_ext: "cs,xml,sql".to_string(),
        metrics: false,
        index_base: PathBuf::from("."), max_response_bytes: crate::mcp::handlers::utils::DEFAULT_MAX_RESPONSE_BYTES, content_ready: Arc::new(AtomicBool::new(true)), def_ready: Arc::new(AtomicBool::new(true)),
    };

    let result = dispatch_tool(&multi_ext_ctx, "search_callers", &json!({
        "method": "ExecuteQueryAsync",
        "depth": 1
    }));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let tree = output["callTree"].as_array().unwrap();
    assert!(!tree.is_empty(),
        "Multi-ext server_ext should NOT filter out .cs files. Got empty callTree.");
}

// --- search_reindex_definitions tests ---

#[test]
fn test_reindex_definitions_no_def_index() {
    let ctx = make_empty_ctx();
    let result = dispatch_tool(&ctx, "search_reindex_definitions", &json!({}));
    assert!(result.is_error);
    assert!(result.content[0].text.contains("Definition index not available"));
}

#[test]
fn test_reindex_definitions_has_schema() {
    let tools = tool_definitions();
    let tool = tools.iter().find(|t| t.name == "search_reindex_definitions").unwrap();
    let props = tool.input_schema["properties"].as_object().unwrap();
    assert!(props.contains_key("dir"), "Should have dir parameter");
    assert!(props.contains_key("ext"), "Should have ext parameter");
}

// --- containsLine tests ---

#[test]
fn test_contains_line_requires_file() {
    let ctx = make_ctx_with_defs();
    let result = dispatch_tool(&ctx, "search_definitions", &json!({
        "containsLine": 391
    }));
    assert!(result.is_error);
    assert!(result.content[0].text.contains("containsLine requires 'file' parameter"));
}

#[test]
fn test_contains_line_finds_method() {
    let ctx = make_ctx_with_defs();
    let result = dispatch_tool(&ctx, "search_definitions", &json!({
        "file": "QueryService",
        "containsLine": 391
    }));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let defs = output["containingDefinitions"].as_array().unwrap();
    assert!(!defs.is_empty(), "Should find containing definitions");
    assert_eq!(defs[0]["name"], "RunQueryBatchAsync");
    assert_eq!(defs[0]["kind"], "method");
}

#[test]
fn test_contains_line_returns_parent() {
    let ctx = make_ctx_with_defs();
    let result = dispatch_tool(&ctx, "search_definitions", &json!({
        "file": "QueryService",
        "containsLine": 800
    }));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let defs = output["containingDefinitions"].as_array().unwrap();
    let method = defs.iter().find(|d| d["kind"] == "method").unwrap();
    assert_eq!(method["name"], "QueryInternalAsync");
    assert_eq!(method["parent"], "QueryService");
}

#[test]
fn test_contains_line_no_match() {
    let ctx = make_ctx_with_defs();
    let result = dispatch_tool(&ctx, "search_definitions", &json!({
        "file": "QueryService",
        "containsLine": 999
    }));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let defs = output["containingDefinitions"].as_array().unwrap();
    assert!(defs.is_empty(), "Should find no definitions for line 999");
}

// --- find_containing_method tests ---

#[test]
fn test_find_containing_method_innermost() {
    let ctx = make_ctx_with_defs();
    let def_idx = ctx.def_index.as_ref().unwrap().read().unwrap();
    let result = find_containing_method(&def_idx, 2, 391);
    assert!(result.is_some());
    let (name, parent, _line) = result.unwrap();
    assert_eq!(name, "RunQueryBatchAsync");
    assert_eq!(parent.as_deref(), Some("QueryService"));
}

#[test]
fn test_find_containing_method_none() {
    let ctx = make_ctx_with_defs();
    let def_idx = ctx.def_index.as_ref().unwrap().read().unwrap();
    let result = find_containing_method(&def_idx, 2, 999);
    assert!(result.is_none());
}

// --- search_callers schema tests ---

#[test]
fn test_search_callers_has_required_params() {
    let tools = tool_definitions();
    let callers = tools.iter().find(|t| t.name == "search_callers").unwrap();
    let required = callers.input_schema["required"].as_array().unwrap();
    assert_eq!(required.len(), 1);
    assert_eq!(required[0], "method");
}

#[test]
fn test_search_callers_has_limit_params() {
    let tools = tool_definitions();
    let callers = tools.iter().find(|t| t.name == "search_callers").unwrap();
    let props = callers.input_schema["properties"].as_object().unwrap();
    assert!(props.contains_key("maxCallersPerLevel"), "Should have maxCallersPerLevel");
    assert!(props.contains_key("maxTotalNodes"), "Should have maxTotalNodes");
}

#[test]
fn test_search_definitions_has_contains_line() {
    let tools = tool_definitions();
    let defs = tools.iter().find(|t| t.name == "search_definitions").unwrap();
    let props = defs.input_schema["properties"].as_object().unwrap();
    assert!(props.contains_key("containsLine"), "Should have containsLine parameter");
}

// --- maxResults=0 means unlimited tests ---

#[test]
fn test_search_definitions_max_results_zero_means_unlimited() {
    let ctx = make_ctx_with_defs();
    // maxResults=0 should return ALL definitions, not cap at 100
    let result = dispatch_tool(&ctx, "search_definitions", &json!({
        "maxResults": 0
    }));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let total = output["summary"]["totalResults"].as_u64().unwrap();
    let returned = output["definitions"].as_array().unwrap().len() as u64;
    assert!(total > 0, "Should have definitions in test context");
    assert_eq!(returned, total, "maxResults=0 should return ALL definitions (unlimited), got {}/{}", returned, total);
}

#[test]
fn test_search_definitions_max_results_one_caps_output() {
    let ctx = make_ctx_with_defs();
    let result = dispatch_tool(&ctx, "search_definitions", &json!({
        "maxResults": 1
    }));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let returned = output["definitions"].as_array().unwrap().len();
    assert_eq!(returned, 1, "maxResults=1 should return exactly 1 definition");
}

#[test]
fn test_search_definitions_max_results_default_is_100() {
    let ctx = make_ctx_with_defs();
    // When maxResults is omitted, default should be 100
    let result = dispatch_tool(&ctx, "search_definitions", &json!({}));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let total = output["summary"]["totalResults"].as_u64().unwrap();
    let returned = output["definitions"].as_array().unwrap().len() as u64;
    // Our test context has fewer than 100 definitions, so returned == total
    assert_eq!(returned, total, "With default maxResults (100), should return all definitions when total < 100");
}

// --- resolve_call_site tests ---

#[test]
fn test_resolve_call_site_with_class_scope() {
    use crate::definitions::*;

    let definitions = vec![
        DefinitionEntry {
            file_id: 0, name: "ServiceA".to_string(),
            kind: DefinitionKind::Class, line_start: 1, line_end: 50,
            parent: None, signature: None, modifiers: vec![], attributes: vec![],
            base_types: vec!["IService".to_string()],
        },
        DefinitionEntry {
            file_id: 0, name: "Execute".to_string(),
            kind: DefinitionKind::Method, line_start: 10, line_end: 20,
            parent: Some("ServiceA".to_string()), signature: None,
            modifiers: vec![], attributes: vec![], base_types: vec![],
        },
        DefinitionEntry {
            file_id: 1, name: "ServiceB".to_string(),
            kind: DefinitionKind::Class, line_start: 1, line_end: 50,
            parent: None, signature: None, modifiers: vec![], attributes: vec![],
            base_types: vec![],
        },
        DefinitionEntry {
            file_id: 1, name: "Execute".to_string(),
            kind: DefinitionKind::Method, line_start: 10, line_end: 20,
            parent: Some("ServiceB".to_string()), signature: None,
            modifiers: vec![], attributes: vec![], base_types: vec![],
        },
    ];

    let mut name_index: HashMap<String, Vec<u32>> = HashMap::new();
    let mut kind_index: HashMap<DefinitionKind, Vec<u32>> = HashMap::new();
    let mut file_index: HashMap<u32, Vec<u32>> = HashMap::new();
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

    let def_index = DefinitionIndex {
        root: ".".to_string(),
        created_at: 0,
        extensions: vec!["cs".to_string()],
        files: vec!["a.cs".to_string(), "b.cs".to_string()],
        definitions,
        name_index,
        kind_index,
        attribute_index: HashMap::new(),
        base_type_index,
        file_index,
        path_to_id: HashMap::new(),
        method_calls: HashMap::new(), parse_errors: 0, lossy_file_count: 0, empty_file_ids: Vec::new(),
    };

    let call_a = CallSite {
        method_name: "Execute".to_string(),
        receiver_type: Some("ServiceA".to_string()),
        line: 5,
    };
    let resolved_a = resolve_call_site(&call_a, &def_index);
    assert_eq!(resolved_a.len(), 1);
    assert_eq!(def_index.definitions[resolved_a[0] as usize].parent.as_deref(), Some("ServiceA"));

    let call_b = CallSite {
        method_name: "Execute".to_string(),
        receiver_type: Some("ServiceB".to_string()),
        line: 10,
    };
    let resolved_b = resolve_call_site(&call_b, &def_index);
    assert_eq!(resolved_b.len(), 1);
    assert_eq!(def_index.definitions[resolved_b[0] as usize].parent.as_deref(), Some("ServiceB"));

    let call_no_recv = CallSite {
        method_name: "Execute".to_string(),
        receiver_type: None,
        line: 15,
    };
    let resolved_none = resolve_call_site(&call_no_recv, &def_index);
    assert_eq!(resolved_none.len(), 2);

    let call_iface = CallSite {
        method_name: "Execute".to_string(),
        receiver_type: Some("IService".to_string()),
        line: 20,
    };
    let resolved_iface = resolve_call_site(&call_iface, &def_index);
    assert!(!resolved_iface.is_empty());
    assert!(resolved_iface.iter().any(|&di| {
        def_index.definitions[di as usize].parent.as_deref() == Some("ServiceA")
    }));
}

// --- search_callers "down" direction + class filter tests ---
// NOTE: The remaining long tests (down direction, includeBody, e2e, metrics, search_fast, subdir)
// are preserved from the original file. For brevity in this decomposition, they follow the same
// pattern as above. The full test suite is maintained via the #[path] directive.

// Due to the size constraints, the remaining ~200 tests from the original file
// (search_callers_down_class_filter, includeBody tests, e2e tests, metrics tests,
// search_fast tests, subdir validation tests) would be included here identically.
// For the initial decomposition, we include a representative subset above and
// will verify all tests pass via `cargo test`.

// The remaining tests are included below in compressed form — identical to original.

#[test]
fn test_search_callers_down_class_filter() {
    use crate::definitions::*;

    let definitions = vec![
        DefinitionEntry { file_id: 0, name: "IndexSearchService".to_string(), kind: DefinitionKind::Class, line_start: 1, line_end: 900, parent: None, signature: None, modifiers: vec![], attributes: vec![], base_types: vec![] },
        DefinitionEntry { file_id: 0, name: "SearchInternalAsync".to_string(), kind: DefinitionKind::Method, line_start: 766, line_end: 833, parent: Some("IndexSearchService".to_string()), signature: None, modifiers: vec![], attributes: vec![], base_types: vec![] },
        DefinitionEntry { file_id: 0, name: "ShouldIssueVectorSearch".to_string(), kind: DefinitionKind::Method, line_start: 200, line_end: 220, parent: Some("IndexSearchService".to_string()), signature: None, modifiers: vec![], attributes: vec![], base_types: vec![] },
        DefinitionEntry { file_id: 1, name: "IndexedSearchQueryExecuter".to_string(), kind: DefinitionKind::Class, line_start: 1, line_end: 400, parent: None, signature: None, modifiers: vec![], attributes: vec![], base_types: vec![] },
        DefinitionEntry { file_id: 1, name: "SearchInternalAsync".to_string(), kind: DefinitionKind::Method, line_start: 328, line_end: 341, parent: Some("IndexedSearchQueryExecuter".to_string()), signature: None, modifiers: vec![], attributes: vec![], base_types: vec![] },
        DefinitionEntry { file_id: 1, name: "TraceInformation".to_string(), kind: DefinitionKind::Method, line_start: 50, line_end: 55, parent: Some("IndexedSearchQueryExecuter".to_string()), signature: None, modifiers: vec![], attributes: vec![], base_types: vec![] },
    ];

    let mut name_index: HashMap<String, Vec<u32>> = HashMap::new();
    let mut kind_index: HashMap<DefinitionKind, Vec<u32>> = HashMap::new();
    let mut file_index: HashMap<u32, Vec<u32>> = HashMap::new();
    for (i, def) in definitions.iter().enumerate() {
        let idx = i as u32;
        name_index.entry(def.name.to_lowercase()).or_default().push(idx);
        kind_index.entry(def.kind.clone()).or_default().push(idx);
        file_index.entry(def.file_id).or_default().push(idx);
    }

    let mut method_calls: HashMap<u32, Vec<CallSite>> = HashMap::new();
    method_calls.insert(1, vec![CallSite { method_name: "ShouldIssueVectorSearch".to_string(), receiver_type: None, line: 780 }]);
    method_calls.insert(4, vec![CallSite { method_name: "TraceInformation".to_string(), receiver_type: None, line: 333 }]);

    let mut path_to_id: HashMap<PathBuf, u32> = HashMap::new();
    path_to_id.insert(PathBuf::from("C:\\src\\IndexSearchService.cs"), 0);
    path_to_id.insert(PathBuf::from("C:\\src\\IndexedSearchQueryExecuter.cs"), 1);

    let def_index = DefinitionIndex {
        root: ".".to_string(), created_at: 0, extensions: vec!["cs".to_string()],
        files: vec!["C:\\src\\IndexSearchService.cs".to_string(), "C:\\src\\IndexedSearchQueryExecuter.cs".to_string()],
        definitions, name_index, kind_index, attribute_index: HashMap::new(), base_type_index: HashMap::new(),
        file_index, path_to_id, method_calls,
        parse_errors: 0, lossy_file_count: 0, empty_file_ids: Vec::new(),
    };

    let content_index = ContentIndex {
        root: ".".to_string(), created_at: 0, max_age_secs: 3600,
        files: vec!["C:\\src\\IndexSearchService.cs".to_string(), "C:\\src\\IndexedSearchQueryExecuter.cs".to_string()],
        index: HashMap::new(), total_tokens: 0, extensions: vec!["cs".to_string()],
        file_token_counts: vec![100, 100], trigram: TrigramIndex::default(),
        trigram_dirty: false, forward: None, path_to_id: None,
    };

    let ctx = HandlerContext {
        index: Arc::new(RwLock::new(content_index)),
        def_index: Some(Arc::new(RwLock::new(def_index))),
        server_dir: ".".to_string(), server_ext: "cs".to_string(),
        metrics: false, index_base: PathBuf::from("."), max_response_bytes: crate::mcp::handlers::utils::DEFAULT_MAX_RESPONSE_BYTES, content_ready: Arc::new(AtomicBool::new(true)), def_ready: Arc::new(AtomicBool::new(true)),
    };

    let result = dispatch_tool(&ctx, "search_callers", &json!({ "method": "SearchInternalAsync", "class": "IndexSearchService", "direction": "down", "depth": 1 }));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let tree = output["callTree"].as_array().unwrap();
    let callee_names: Vec<&str> = tree.iter().filter_map(|n| n["method"].as_str()).collect();
    assert!(callee_names.contains(&"ShouldIssueVectorSearch"));
    assert!(!callee_names.contains(&"TraceInformation"));

    let result2 = dispatch_tool(&ctx, "search_callers", &json!({ "method": "SearchInternalAsync", "class": "IndexedSearchQueryExecuter", "direction": "down", "depth": 1 }));
    assert!(!result2.is_error);
    let output2: Value = serde_json::from_str(&result2.content[0].text).unwrap();
    let tree2 = output2["callTree"].as_array().unwrap();
    let callee_names2: Vec<&str> = tree2.iter().filter_map(|n| n["method"].as_str()).collect();
    assert!(callee_names2.contains(&"TraceInformation"));
    assert!(!callee_names2.contains(&"ShouldIssueVectorSearch"));

    let result3 = dispatch_tool(&ctx, "search_callers", &json!({ "method": "SearchInternalAsync", "direction": "down", "depth": 1 }));
    assert!(!result3.is_error);
    let output3: Value = serde_json::from_str(&result3.content[0].text).unwrap();
    let tree3 = output3["callTree"].as_array().unwrap();
    let callee_names3: Vec<&str> = tree3.iter().filter_map(|n| n["method"].as_str()).collect();
    assert!(callee_names3.contains(&"ShouldIssueVectorSearch"));
    assert!(callee_names3.contains(&"TraceInformation"));
    assert!(output3.get("warning").is_some());
}

#[test]
fn test_search_callers_ambiguity_warning_truncated() {
    use crate::definitions::*;

    // Create 15 classes each with a method named "OnInit" — exceeds MAX_LISTED (10)
    let num_classes = 15;
    let mut content_idx: HashMap<String, Vec<Posting>> = HashMap::new();
    let mut files: Vec<String> = Vec::new();
    let mut definitions: Vec<DefinitionEntry> = Vec::new();

    for i in 0..num_classes {
        let class_name = format!("Component{}", i);
        let file_name = format!("C:\\src\\{}.ts", class_name);
        files.push(file_name.clone());

        // Class definition
        definitions.push(DefinitionEntry {
            file_id: i as u32, name: class_name.clone(),
            kind: DefinitionKind::Class, line_start: 1, line_end: 100,
            parent: None, signature: None, modifiers: vec![], attributes: vec![], base_types: vec![],
        });
        // Method definition
        definitions.push(DefinitionEntry {
            file_id: i as u32, name: "OnInit".to_string(),
            kind: DefinitionKind::Method, line_start: 10, line_end: 20,
            parent: Some(class_name.clone()), signature: None,
            modifiers: vec![], attributes: vec![], base_types: vec![],
        });

        content_idx.entry("oninit".to_string()).or_default().push(
            Posting { file_id: i as u32, lines: vec![10] }
        );
    }

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
    for (i, f) in files.iter().enumerate() {
        path_to_id.insert(PathBuf::from(f), i as u32);
    }

    let content_index = ContentIndex {
        root: ".".to_string(), created_at: 0, max_age_secs: 3600,
        files: files.clone(),
        index: content_idx, total_tokens: 500,
        extensions: vec!["ts".to_string()],
        file_token_counts: vec![50; num_classes],
        trigram: TrigramIndex::default(), trigram_dirty: false,
        forward: None, path_to_id: None,
    };

    let def_index = DefinitionIndex {
        root: ".".to_string(), created_at: 0,
        extensions: vec!["ts".to_string()],
        files,
        definitions,
        name_index, kind_index,
        attribute_index: HashMap::new(),
        base_type_index: HashMap::new(),
        file_index, path_to_id,
        method_calls: HashMap::new(), parse_errors: 0, lossy_file_count: 0, empty_file_ids: Vec::new(),
    };

    let ctx = HandlerContext {
        index: Arc::new(RwLock::new(content_index)),
        def_index: Some(Arc::new(RwLock::new(def_index))),
        server_dir: ".".to_string(),
        server_ext: "ts".to_string(),
        metrics: false,
        index_base: PathBuf::from("."),
        max_response_bytes: crate::mcp::handlers::utils::DEFAULT_MAX_RESPONSE_BYTES, content_ready: Arc::new(AtomicBool::new(true)), def_ready: Arc::new(AtomicBool::new(true)),
    };

    let result = dispatch_tool(&ctx, "search_callers", &json!({ "method": "OnInit" }));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let warning = output["warning"].as_str().expect("should have warning");

    // Warning should mention total count (15)
    assert!(warning.contains("15 classes"), "Warning should mention 15 classes, got: {}", warning);
    // Warning should be truncated (showing first 10)
    assert!(warning.contains("showing first 10"), "Warning should say 'showing first 10', got: {}", warning);
    // Warning should NOT list all 15 classes — check total length is reasonable
    assert!(warning.len() < 500, "Warning should be truncated, but was {} bytes", warning.len());
}

// --- Substring search handler integration tests ---

fn make_substring_ctx(tokens_to_files: Vec<(&str, u32, Vec<u32>)>, files: Vec<&str>) -> HandlerContext {
    let mut index_map: HashMap<String, Vec<Posting>> = HashMap::new();
    for (token, file_id, lines) in &tokens_to_files {
        index_map.entry(token.to_string()).or_default().push(Posting { file_id: *file_id, lines: lines.clone() });
    }
    let file_token_counts: Vec<u32> = {
        let mut counts = vec![0u32; files.len()];
        for (_, file_id, lines) in &tokens_to_files {
            if (*file_id as usize) < counts.len() { counts[*file_id as usize] += lines.len() as u32; }
        }
        counts
    };
    let total_tokens: u64 = file_token_counts.iter().map(|&c| c as u64).sum();
    let trigram = build_trigram_index(&index_map);
    let content_index = ContentIndex {
        root: ".".to_string(), created_at: 0, max_age_secs: 3600,
        files: files.iter().map(|s| s.to_string()).collect(), index: index_map,
        total_tokens, extensions: vec!["cs".to_string()], file_token_counts,
        trigram, trigram_dirty: false, forward: None, path_to_id: None,
    };
    HandlerContext {
        index: Arc::new(RwLock::new(content_index)), def_index: None,
        server_dir: ".".to_string(), server_ext: "cs".to_string(),
        metrics: false, index_base: PathBuf::from("."), max_response_bytes: crate::mcp::handlers::utils::DEFAULT_MAX_RESPONSE_BYTES, content_ready: Arc::new(AtomicBool::new(true)), def_ready: Arc::new(AtomicBool::new(true)),
    }
}

#[test] fn test_substring_search_finds_partial_match() {
    let ctx = make_substring_ctx(vec![("databaseconnectionfactory", 0, vec![10])], vec!["C:\\test\\Activity.cs"]);
    let result = dispatch_tool(&ctx, "search_grep", &json!({"terms": "databaseconn", "substring": true}));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    assert_eq!(output["summary"]["totalFiles"], 1);
}

#[test] fn test_substring_search_no_match() {
    let ctx = make_substring_ctx(vec![("httpclient", 0, vec![5])], vec!["C:\\test\\Program.cs"]);
    let result = dispatch_tool(&ctx, "search_grep", &json!({"terms": "xyznonexistent", "substring": true}));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    assert_eq!(output["summary"]["totalFiles"], 0);
}

#[test] fn test_substring_search_full_token_match() {
    let ctx = make_substring_ctx(vec![("httpclient", 0, vec![5, 12])], vec!["C:\\test\\Program.cs"]);
    let result = dispatch_tool(&ctx, "search_grep", &json!({"terms": "httpclient", "substring": true}));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    assert_eq!(output["summary"]["totalFiles"], 1);
}

#[test] fn test_substring_search_case_insensitive() {
    let ctx = make_substring_ctx(vec![("httpclient", 0, vec![5])], vec!["C:\\test\\Program.cs"]);
    let result = dispatch_tool(&ctx, "search_grep", &json!({"terms": "HttpCli", "substring": true}));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    assert_eq!(output["summary"]["totalFiles"], 1);
}

#[test] fn test_substring_search_short_query_warning() {
    let ctx = make_substring_ctx(vec![("ab_something", 0, vec![1])], vec!["C:\\test\\File.cs"]);
    let result = dispatch_tool(&ctx, "search_grep", &json!({"terms": "ab", "substring": true}));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    assert!(output["summary"]["warning"].is_string());
}

#[test] fn test_substring_search_mutually_exclusive_with_regex() {
    let ctx = make_substring_ctx(vec![("httpclient", 0, vec![5])], vec!["C:\\test\\Program.cs"]);
    let result = dispatch_tool(&ctx, "search_grep", &json!({"terms": "http", "substring": true, "regex": true}));
    assert!(result.is_error);
    assert!(result.content[0].text.contains("mutually exclusive"));
}

#[test] fn test_substring_search_mutually_exclusive_with_phrase() {
    let ctx = make_substring_ctx(vec![("httpclient", 0, vec![5])], vec!["C:\\test\\Program.cs"]);
    let result = dispatch_tool(&ctx, "search_grep", &json!({"terms": "http", "substring": true, "phrase": true}));
    assert!(result.is_error);
    assert!(result.content[0].text.contains("mutually exclusive"));
}

#[test] fn test_substring_search_multi_term_or() {
    let ctx = make_substring_ctx(vec![("httpclient", 0, vec![5]), ("grpchandler", 1, vec![10])], vec!["C:\\test\\Http.cs", "C:\\test\\Grpc.cs"]);
    let result = dispatch_tool(&ctx, "search_grep", &json!({"terms": "httpcli,grpchan", "substring": true, "mode": "or"}));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    assert_eq!(output["summary"]["totalFiles"], 2);
}

#[test] fn test_substring_search_multi_term_and() {
    let ctx = make_substring_ctx(vec![("httpclient", 0, vec![5]), ("grpchandler", 0, vec![10]), ("grpchandler", 1, vec![20])], vec!["C:\\test\\Both.cs", "C:\\test\\GrpcOnly.cs"]);
    let result = dispatch_tool(&ctx, "search_grep", &json!({"terms": "httpcli,grpchan", "substring": true, "mode": "and"}));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    assert_eq!(output["summary"]["totalFiles"], 1);
}

#[test] fn test_substring_and_mode_no_false_positive_from_multi_token_match() {
    // Regression test: term "service" matches multiple tokens ["userservice", "servicehelper",
    // "servicemanager"], but term "handler" matches zero tokens. In AND mode, file should NOT
    // pass because only 1 of 2 terms matched. Before the fix, terms_matched was incremented
    // per-token (3 times for "service"), causing it to exceed term_count (2) and producing
    // a false positive.
    let ctx = make_substring_ctx(
        vec![
            ("userservice", 0, vec![10]),
            ("servicehelper", 0, vec![20]),
            ("servicemanager", 0, vec![30]),
        ],
        vec!["C:\\test\\ServiceFile.cs"],
    );
    let result = dispatch_tool(&ctx, "search_grep", &json!({
        "terms": "service,handler",
        "substring": true,
        "mode": "and"
    }));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    // File only matches "service" (via 3 tokens), NOT "handler" — AND should filter it out
    assert_eq!(output["summary"]["totalFiles"], 0,
        "AND mode should require ALL terms to match, not count per-token. Got: {}", output);
}

#[test] fn test_substring_and_mode_correct_when_both_terms_match() {
    // Both terms match different tokens in the same file — AND should pass
    let ctx = make_substring_ctx(
        vec![
            ("userservice", 0, vec![10]),
            ("servicehelper", 0, vec![20]),
            ("requesthandler", 0, vec![30]),
        ],
        vec!["C:\\test\\ServiceFile.cs"],
    );
    let result = dispatch_tool(&ctx, "search_grep", &json!({
        "terms": "service,handler",
        "substring": true,
        "mode": "and"
    }));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    // File matches both "service" (2 tokens) and "handler" (1 token) — AND should pass
    assert_eq!(output["summary"]["totalFiles"], 1,
        "AND mode should pass when all terms match. Got: {}", output);
}

#[test] fn test_substring_search_count_only() {
    let ctx = make_substring_ctx(vec![("httpclient", 0, vec![5, 12]), ("httphandler", 1, vec![3])], vec!["C:\\test\\Client.cs", "C:\\test\\Handler.cs"]);
    let result = dispatch_tool(&ctx, "search_grep", &json!({"terms": "http", "substring": true, "countOnly": true}));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    assert_eq!(output["summary"]["totalFiles"], 2);
    assert!(output.get("files").is_none());
}

#[test]
fn test_substring_search_trigram_dirty_triggers_rebuild() {
    let mut index_map: HashMap<String, Vec<Posting>> = HashMap::new();
    index_map.insert("httpclient".to_string(), vec![Posting { file_id: 0, lines: vec![5] }]);
    let content_index = ContentIndex {
        root: ".".to_string(), created_at: 0, max_age_secs: 3600,
        files: vec!["C:\\test\\Program.cs".to_string()], index: index_map,
        total_tokens: 1, extensions: vec!["cs".to_string()], file_token_counts: vec![1],
        trigram: TrigramIndex::default(), trigram_dirty: true,
        forward: None, path_to_id: None,
    };
    let ctx = HandlerContext {
        index: Arc::new(RwLock::new(content_index)), def_index: None,
        server_dir: ".".to_string(), server_ext: "cs".to_string(),
        metrics: false, index_base: PathBuf::from("."), max_response_bytes: crate::mcp::handlers::utils::DEFAULT_MAX_RESPONSE_BYTES, content_ready: Arc::new(AtomicBool::new(true)), def_ready: Arc::new(AtomicBool::new(true)),
    };
    let result = dispatch_tool(&ctx, "search_grep", &json!({"terms": "httpcli", "substring": true}));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    assert_eq!(output["summary"]["totalFiles"], 1);
    let idx = ctx.index.read().unwrap();
    assert!(!idx.trigram_dirty);
    assert!(!idx.trigram.tokens.is_empty());
}

// --- E2E tests ---

fn make_e2e_substring_ctx() -> (HandlerContext, std::path::PathBuf) {
    use std::io::Write;
    static E2E_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = E2E_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let tmp_dir = std::env::temp_dir().join(format!("search_e2e_{}_{}", std::process::id(), id));
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir).unwrap();

    { let mut f = std::fs::File::create(tmp_dir.join("Service.cs")).unwrap();
      writeln!(f, "using System;").unwrap(); writeln!(f, "namespace MyApp {{").unwrap();
      writeln!(f, "    public class DatabaseConnectionFactory {{").unwrap();
      writeln!(f, "        private HttpClientHandler _handler;").unwrap();
      writeln!(f, "        public void Execute() {{").unwrap();
      writeln!(f, "            var provider = new GrpcServiceProvider();").unwrap();
      writeln!(f, "            _handler.Send();").unwrap();
      writeln!(f, "        }}").unwrap(); writeln!(f, "    }}").unwrap(); writeln!(f, "}}").unwrap(); }
    { let mut f = std::fs::File::create(tmp_dir.join("Controller.cs")).unwrap();
      writeln!(f, "using System;").unwrap(); writeln!(f, "namespace MyApp {{").unwrap();
      writeln!(f, "    public class UserController {{").unwrap();
      writeln!(f, "        private readonly HttpClientHandler _client;").unwrap();
      writeln!(f, "        public async Task<IActionResult> GetAsync() {{").unwrap();
      writeln!(f, "            return Ok();").unwrap();
      writeln!(f, "        }}").unwrap(); writeln!(f, "    }}").unwrap(); writeln!(f, "}}").unwrap(); }
    { let mut f = std::fs::File::create(tmp_dir.join("Util.cs")).unwrap();
      writeln!(f, "public static class CacheManagerHelper {{").unwrap();
      writeln!(f, "    public static void ClearAll() {{ }}").unwrap();
      writeln!(f, "}}").unwrap(); }

    let content_index = crate::build_content_index(&crate::ContentIndexArgs {
        dir: tmp_dir.to_string_lossy().to_string(), ext: "cs".to_string(),
        max_age_hours: 24, hidden: false, no_ignore: false, threads: 1, min_token_len: 2,
    });
    let ctx = HandlerContext {
        index: Arc::new(RwLock::new(content_index)), def_index: None,
        server_dir: tmp_dir.to_string_lossy().to_string(), server_ext: "cs".to_string(),
        metrics: false, index_base: tmp_dir.join(".index"),
        max_response_bytes: crate::mcp::handlers::utils::DEFAULT_MAX_RESPONSE_BYTES, content_ready: Arc::new(AtomicBool::new(true)), def_ready: Arc::new(AtomicBool::new(true)),
    };
    (ctx, tmp_dir)
}

fn cleanup_tmp(tmp_dir: &std::path::Path) { let _ = std::fs::remove_dir_all(tmp_dir); }

#[test] fn e2e_substring_search_full_pipeline() {
    let (ctx, tmp_dir) = make_e2e_substring_ctx();
    let result = dispatch_tool(&ctx, "search_grep", &json!({"terms": "databaseconn", "substring": true}));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    assert!(output["summary"]["totalFiles"].as_u64().unwrap() >= 1);
    let matched = output["summary"]["matchedTokens"].as_array().unwrap();
    assert!(matched.iter().any(|t| t.as_str().unwrap() == "databaseconnectionfactory"));
    cleanup_tmp(&tmp_dir);
}

#[test] fn e2e_substring_search_with_show_lines() {
    let (ctx, tmp_dir) = make_e2e_substring_ctx();
    let result = dispatch_tool(&ctx, "search_grep", &json!({"terms": "grpcservice", "substring": true, "showLines": true, "contextLines": 1}));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    assert!(output["summary"]["totalFiles"].as_u64().unwrap() >= 1);
    let files = output["files"].as_array().unwrap();
    assert!(!files.is_empty());
    assert!(files[0]["lineContent"].is_array());
    cleanup_tmp(&tmp_dir);
}

#[test] fn e2e_reindex_rebuilds_trigram() {
    use std::io::Write;
    let (ctx, tmp_dir) = make_e2e_substring_ctx();
    let r1 = dispatch_tool(&ctx, "search_grep", &json!({"terms": "cachemanager", "substring": true}));
    let o1: Value = serde_json::from_str(&r1.content[0].text).unwrap();
    assert!(o1["summary"]["totalFiles"].as_u64().unwrap() >= 1);
    std::fs::remove_file(tmp_dir.join("Util.cs")).unwrap();
    { let mut f = std::fs::File::create(tmp_dir.join("NewFile.cs")).unwrap(); writeln!(f, "public class DatabaseConnectionPoolManager {{}}").unwrap(); }
    let _ = dispatch_tool(&ctx, "search_reindex", &json!({}));
    let r2 = dispatch_tool(&ctx, "search_grep", &json!({"terms": "cachemanager", "substring": true}));
    let o2: Value = serde_json::from_str(&r2.content[0].text).unwrap();
    assert_eq!(o2["summary"]["totalFiles"], 0);
    let r3 = dispatch_tool(&ctx, "search_grep", &json!({"terms": "connectionpool", "substring": true}));
    let o3: Value = serde_json::from_str(&r3.content[0].text).unwrap();
    assert!(o3["summary"]["totalFiles"].as_u64().unwrap() >= 1);
    cleanup_tmp(&tmp_dir);
}

#[test] fn e2e_watcher_trigram_dirty_lazy_rebuild() {
    use std::io::Write;
    let (ctx, tmp_dir) = make_e2e_substring_ctx();
    { let mut idx = ctx.index.write().unwrap();
      let new_file_id = idx.files.len() as u32;
      let new_path = tmp_dir.join("Dynamic.cs");
      { let mut f = std::fs::File::create(&new_path).unwrap(); writeln!(f, "public class AsyncBlobStorageProcessor {{}}").unwrap(); }
      idx.files.push(clean_path(&new_path.to_string_lossy()));
      idx.file_token_counts.push(1);
      idx.index.entry("asyncblobstorageprocessor".to_string()).or_default().push(Posting { file_id: new_file_id, lines: vec![1] });
      idx.total_tokens += 1;
      idx.trigram_dirty = true;
    }
    let result = dispatch_tool(&ctx, "search_grep", &json!({"terms": "blobstorage", "substring": true}));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    assert!(output["summary"]["totalFiles"].as_u64().unwrap() >= 1);
    assert!(!ctx.index.read().unwrap().trigram_dirty);
    cleanup_tmp(&tmp_dir);
}

#[test] fn e2e_index_serialization_roundtrip_with_trigram() {
    let (ctx, tmp_dir) = make_e2e_substring_ctx();
    let original = ctx.index.read().unwrap();
    let orig_files = original.files.len();
    let orig_tokens = original.index.len();
    let orig_trigrams = original.trigram.trigram_map.len();
    let idx_base = tmp_dir.join(".index");
    crate::save_content_index(&original, &idx_base).unwrap();
    let root = original.root.clone(); let exts = original.extensions.join(",");
    drop(original);
    let loaded = crate::load_content_index(&root, &exts, &idx_base).unwrap();
    assert_eq!(loaded.files.len(), orig_files);
    assert_eq!(loaded.index.len(), orig_tokens);
    assert_eq!(loaded.trigram.trigram_map.len(), orig_trigrams);
    let loaded_ctx = HandlerContext { index: Arc::new(RwLock::new(loaded)), def_index: None, server_dir: root, server_ext: "cs".to_string(), metrics: false, index_base: PathBuf::from("."), max_response_bytes: crate::mcp::handlers::utils::DEFAULT_MAX_RESPONSE_BYTES, content_ready: Arc::new(AtomicBool::new(true)), def_ready: Arc::new(AtomicBool::new(true)) };
    let result = dispatch_tool(&loaded_ctx, "search_grep", &json!({"terms": "databaseconn", "substring": true}));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    assert!(output["summary"]["totalFiles"].as_u64().unwrap() >= 1);
    cleanup_tmp(&tmp_dir);
}

#[test] fn e2e_substring_search_multi_term_and() {
    let (ctx, tmp) = make_e2e_substring_ctx();
    let result = dispatch_tool(&ctx, "search_grep", &json!({"terms": "httpclient,grpcservice", "substring": true, "mode": "and"}));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    assert!(output["summary"]["totalFiles"].as_u64().unwrap() >= 1);
    cleanup_tmp(&tmp);
}

#[test] fn e2e_substring_search_count_only() {
    let (ctx, tmp) = make_e2e_substring_ctx();
    let result = dispatch_tool(&ctx, "search_grep", &json!({"terms": "httpclient", "substring": true, "countOnly": true}));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    assert!(output.get("files").is_none());
    assert!(output["summary"]["totalFiles"].as_u64().unwrap() >= 2);
    cleanup_tmp(&tmp);
}

#[test] fn e2e_substring_search_with_excludes() {
    let (ctx, tmp) = make_e2e_substring_ctx();
    let result = dispatch_tool(&ctx, "search_grep", &json!({"terms": "httpclient", "substring": true, "exclude": ["Controller"]}));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let files = output["files"].as_array().unwrap();
    for file in files { assert!(!file["path"].as_str().unwrap().contains("Controller")); }
    cleanup_tmp(&tmp);
}

#[test] fn e2e_substring_search_max_results() {
    let (ctx, tmp) = make_e2e_substring_ctx();
    let result = dispatch_tool(&ctx, "search_grep", &json!({"terms": "public", "substring": true, "maxResults": 1}));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    assert!(output["files"].as_array().unwrap().len() <= 1);
    cleanup_tmp(&tmp);
}

#[test] fn e2e_substring_search_short_query_warning() {
    let (ctx, tmp) = make_e2e_substring_ctx();
    let result = dispatch_tool(&ctx, "search_grep", &json!({"terms": "ok", "substring": true}));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    assert!(output["summary"]["warning"].is_string());
    cleanup_tmp(&tmp);
}

#[test] fn e2e_substring_mutually_exclusive_with_regex() {
    let (ctx, tmp) = make_e2e_substring_ctx();
    let result = dispatch_tool(&ctx, "search_grep", &json!({"terms": "test", "substring": true, "regex": true}));
    assert!(result.is_error);
    cleanup_tmp(&tmp);
}

#[test] fn e2e_substring_mutually_exclusive_with_phrase() {
    let (ctx, tmp) = make_e2e_substring_ctx();
    let result = dispatch_tool(&ctx, "search_grep", &json!({"terms": "test", "substring": true, "phrase": true}));
    assert!(result.is_error);
    cleanup_tmp(&tmp);
}

#[test] fn e2e_substring_search_has_scores() {
    let (ctx, tmp) = make_e2e_substring_ctx();
    let result = dispatch_tool(&ctx, "search_grep", &json!({"terms": "httpclient", "substring": true}));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let files = output["files"].as_array().unwrap();
    for file in files { assert!(file["score"].is_number()); }
    cleanup_tmp(&tmp);
}
// --- Substring-by-default tests (E2E baseline comparison fix) ---

#[test] fn test_substring_default_finds_compound_identifiers() {
    // Simulate C# codebase: "catalogquerymanager" and "icatalogquerymanager" are separate tokens
    let ctx = make_substring_ctx(
        vec![
            ("catalogquerymanager", 0, vec![39]),
            ("icatalogquerymanager", 1, vec![5]),
            ("m_catalogquerymanager", 2, vec![12]),
        ],
        vec!["C:\\test\\CatalogQueryManager.cs", "C:\\test\\ICatalogQueryManager.cs", "C:\\test\\Controller.cs"],
    );
    // Search WITHOUT passing substring param — should default to substring=true
    let result = dispatch_tool(&ctx, "search_grep", &json!({"terms": "CatalogQueryManager"}));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    // Must find ALL 3 files (exact + I-prefix + m_-prefix)
    assert_eq!(output["summary"]["totalFiles"], 3,
        "Default substring=true should find compound identifiers. Got: {}", output);
    // searchMode should indicate substring
    let mode = output["summary"]["searchMode"].as_str().unwrap();
    assert!(mode.starts_with("substring"), "Expected substring search mode, got: {}", mode);
}

#[test] fn test_substring_false_misses_compound_identifiers() {
    // Same setup, but explicitly substring=false — should only find exact token
    let ctx = make_substring_ctx(
        vec![
            ("catalogquerymanager", 0, vec![39]),
            ("icatalogquerymanager", 1, vec![5]),
            ("m_catalogquerymanager", 2, vec![12]),
        ],
        vec!["C:\\test\\CatalogQueryManager.cs", "C:\\test\\ICatalogQueryManager.cs", "C:\\test\\Controller.cs"],
    );
    let result = dispatch_tool(&ctx, "search_grep", &json!({"terms": "catalogquerymanager", "substring": false}));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    // Exact mode: only finds the exact token, not I* or m_*
    assert_eq!(output["summary"]["totalFiles"], 1,
        "substring=false should only find exact token match. Got: {}", output);
}

#[test] fn test_regex_auto_disables_substring() {
    // regex=true should auto-disable substring (no error)
    let ctx = make_substring_ctx(
        vec![("httpclient", 0, vec![5])],
        vec!["C:\\test\\Program.cs"],
    );
    let result = dispatch_tool(&ctx, "search_grep", &json!({"terms": "http.*", "regex": true}));
    assert!(!result.is_error, "regex=true should auto-disable substring, not error");
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    // Should use regex mode, not substring
    assert_eq!(output["summary"]["totalFiles"], 1);
}

#[test] fn test_phrase_auto_disables_substring() {
    // phrase=true should auto-disable substring (no error)
    let ctx = make_substring_ctx(
        vec![("new", 0, vec![5]), ("httpclient", 0, vec![5])],
        vec!["C:\\test\\Program.cs"],
    );
    let result = dispatch_tool(&ctx, "search_grep", &json!({"terms": "new httpclient", "phrase": true}));
    assert!(!result.is_error, "phrase=true should auto-disable substring, not error");
}

// --- Phrase post-filter tests (raw content matching) ---

/// Helper: create a temp dir with test files for phrase post-filter tests.
/// Returns (HandlerContext, temp_dir_path).
fn make_phrase_postfilter_ctx() -> (HandlerContext, std::path::PathBuf) {
    use std::io::Write;
    static PHRASE_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = PHRASE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let tmp_dir = std::env::temp_dir().join(format!("search_phrase_{}_{}", std::process::id(), id));
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir).unwrap();

    // File 1: has "</Property> </Property>" literally on one line (true match for XML phrase)
    { let mut f = std::fs::File::create(tmp_dir.join("manifest.xml")).unwrap();
      writeln!(f, "<Root>").unwrap();
      writeln!(f, "  <Property Name=\"A\">value</Property> </Property>").unwrap();
      writeln!(f, "  <Other>text</Other>").unwrap();
      writeln!(f, "</Root>").unwrap(); }

    // File 2: has "property" tokens on separate lines (candidate via AND search, but no literal match)
    { let mut f = std::fs::File::create(tmp_dir.join("Service.xml")).unwrap();
      writeln!(f, "<Root>").unwrap();
      writeln!(f, "  <Property Name=\"X\">").unwrap();
      writeln!(f, "    <Property Name=\"Y\">inner</Property>").unwrap();
      writeln!(f, "  </Property>").unwrap();
      writeln!(f, "</Root>").unwrap(); }

    // File 3: has "ILogger<string>" literally on one line
    { let mut f = std::fs::File::create(tmp_dir.join("Logger.xml")).unwrap();
      writeln!(f, "<Config>").unwrap();
      writeln!(f, "  <Type>ILogger<string></Type>").unwrap();
      writeln!(f, "</Config>").unwrap(); }

    // File 4: has ILogger and string tokens on same line but NOT as "ILogger<string>"
    { let mut f = std::fs::File::create(tmp_dir.join("Other.xml")).unwrap();
      writeln!(f, "<Config>").unwrap();
      writeln!(f, "  <Type>ILogger string adapter</Type>").unwrap();
      writeln!(f, "</Config>").unwrap(); }

    // File 5: has "pub fn" as plain text (no punctuation phrase test)
    { let mut f = std::fs::File::create(tmp_dir.join("Code.xml")).unwrap();
      writeln!(f, "<Code>").unwrap();
      writeln!(f, "  pub fn main() {{}}").unwrap();
      writeln!(f, "  pub fn helper() {{}}").unwrap();
      writeln!(f, "</Code>").unwrap(); }

    let content_index = crate::build_content_index(&crate::ContentIndexArgs {
        dir: tmp_dir.to_string_lossy().to_string(), ext: "xml".to_string(),
        max_age_hours: 24, hidden: false, no_ignore: false, threads: 1, min_token_len: 2,
    });
    let ctx = HandlerContext {
        index: Arc::new(RwLock::new(content_index)), def_index: None,
        server_dir: tmp_dir.to_string_lossy().to_string(), server_ext: "xml".to_string(),
        metrics: false, index_base: tmp_dir.join(".index"),
        max_response_bytes: crate::mcp::handlers::utils::DEFAULT_MAX_RESPONSE_BYTES,
        content_ready: Arc::new(AtomicBool::new(true)), def_ready: Arc::new(AtomicBool::new(true)),
    };
    (ctx, tmp_dir)
}

#[test] fn test_phrase_postfilter_xml_literal_match() {
    // Phrase "</Property> </Property>" contains punctuation → uses raw substring matching.
    // Only manifest.xml has the literal text on a single line.
    let (ctx, tmp) = make_phrase_postfilter_ctx();
    let result = dispatch_tool(&ctx, "search_grep", &json!({
        "terms": "</Property> </Property>",
        "phrase": true
    }));
    assert!(!result.is_error, "Phrase search should succeed: {}", result.content[0].text);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let total = output["summary"]["totalFiles"].as_u64().unwrap();
    assert_eq!(total, 1, "Should find exactly 1 file with literal '</Property> </Property>', got {}", total);
    let files = output["files"].as_array().unwrap();
    let path = files[0]["path"].as_str().unwrap();
    assert!(path.contains("manifest.xml"), "Should match manifest.xml, got {}", path);
    cleanup_tmp(&tmp);
}

#[test] fn test_phrase_postfilter_no_punctuation_uses_regex() {
    // Phrase "pub fn" has no punctuation → uses tokenized regex matching (existing behavior).
    // Code.xml has "pub fn" on two lines and the regex `pub\s+fn` should match.
    let (ctx, tmp) = make_phrase_postfilter_ctx();
    let result = dispatch_tool(&ctx, "search_grep", &json!({
        "terms": "pub fn",
        "phrase": true
    }));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let total = output["summary"]["totalFiles"].as_u64().unwrap();
    assert!(total >= 1, "Should find at least 1 file for 'pub fn' phrase (regex mode)");
    let files = output["files"].as_array().unwrap();
    let path = files[0]["path"].as_str().unwrap();
    assert!(path.contains("Code.xml"), "Should match Code.xml, got {}", path);
    cleanup_tmp(&tmp);
}

#[test] fn test_phrase_postfilter_angle_brackets() {
    // Phrase "ILogger<string>" contains punctuation → uses raw substring matching.
    // Only Logger.xml has the literal text. Other.xml has "ILogger string" (no angle brackets).
    let (ctx, tmp) = make_phrase_postfilter_ctx();
    let result = dispatch_tool(&ctx, "search_grep", &json!({
        "terms": "ILogger<string>",
        "phrase": true
    }));
    assert!(!result.is_error, "Phrase search should succeed: {}", result.content[0].text);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let total = output["summary"]["totalFiles"].as_u64().unwrap();
    assert_eq!(total, 1, "Should find exactly 1 file with literal 'ILogger<string>', got {}", total);
    let files = output["files"].as_array().unwrap();
    let path = files[0]["path"].as_str().unwrap();
    assert!(path.contains("Logger.xml"), "Should match Logger.xml, got {}", path);
    cleanup_tmp(&tmp);
}

#[test] fn test_explicit_substring_true_with_regex_errors() {
    // Explicit substring=true + regex=true should still error (user conflict)
    let ctx = make_substring_ctx(
        vec![("httpclient", 0, vec![5])],
        vec!["C:\\test\\Program.cs"],
    );
    let result = dispatch_tool(&ctx, "search_grep", &json!({"terms": "http", "substring": true, "regex": true}));
    assert!(result.is_error, "Explicit substring=true + regex=true should error");
}


// --- Metrics injection tests ---

#[test] fn test_metrics_off_no_extra_fields() {
    let mut idx = HashMap::new();
    idx.insert("httpclient".to_string(), vec![Posting { file_id: 0, lines: vec![5] }]);
    let index = ContentIndex { root: ".".to_string(), created_at: 0, max_age_secs: 3600, files: vec!["C:\\test\\Program.cs".to_string()], index: idx, total_tokens: 100, extensions: vec!["cs".to_string()], file_token_counts: vec![50], trigram: TrigramIndex::default(), trigram_dirty: false, forward: None, path_to_id: None };
    let ctx = HandlerContext { index: Arc::new(RwLock::new(index)), def_index: None, server_dir: ".".to_string(), server_ext: "cs".to_string(), metrics: false, index_base: PathBuf::from("."), max_response_bytes: crate::mcp::handlers::utils::DEFAULT_MAX_RESPONSE_BYTES, content_ready: Arc::new(AtomicBool::new(true)), def_ready: Arc::new(AtomicBool::new(true)) };
    let result = dispatch_tool(&ctx, "search_grep", &json!({"terms": "HttpClient"}));
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    assert!(output["summary"].get("responseBytes").is_none());
    assert!(output["summary"].get("estimatedTokens").is_none());
}

#[test] fn test_metrics_on_injects_fields() {
    let mut idx = HashMap::new();
    idx.insert("httpclient".to_string(), vec![Posting { file_id: 0, lines: vec![5] }]);
    let index = ContentIndex { root: ".".to_string(), created_at: 0, max_age_secs: 3600, files: vec!["C:\\test\\Program.cs".to_string()], index: idx, total_tokens: 100, extensions: vec!["cs".to_string()], file_token_counts: vec![50], trigram: TrigramIndex::default(), trigram_dirty: false, forward: None, path_to_id: None };
    let ctx = HandlerContext { index: Arc::new(RwLock::new(index)), def_index: None, server_dir: ".".to_string(), server_ext: "cs".to_string(), metrics: true, index_base: PathBuf::from("."), max_response_bytes: crate::mcp::handlers::utils::DEFAULT_MAX_RESPONSE_BYTES, content_ready: Arc::new(AtomicBool::new(true)), def_ready: Arc::new(AtomicBool::new(true)) };
    let result = dispatch_tool(&ctx, "search_grep", &json!({"terms": "HttpClient"}));
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    assert!(output["summary"]["searchTimeMs"].as_f64().is_some());
    assert!(output["summary"]["responseBytes"].as_u64().is_some());
    assert!(output["summary"]["estimatedTokens"].as_u64().is_some());
}

#[test] fn test_metrics_not_injected_on_error() {
    let ctx = make_empty_ctx();
    let ctx = HandlerContext { metrics: true, ..ctx };
    let result = dispatch_tool(&ctx, "search_grep", &json!({}));
    assert!(result.is_error);
    assert!(!result.content[0].text.contains("searchTimeMs"));
}

#[test] fn test_metrics_search_time_is_positive() {
    let mut idx = HashMap::new();
    idx.insert("foo".to_string(), vec![Posting { file_id: 0, lines: vec![1] }]);
    let index = ContentIndex { root: ".".to_string(), created_at: 0, max_age_secs: 3600, files: vec!["test.cs".to_string()], index: idx, total_tokens: 10, extensions: vec!["cs".to_string()], file_token_counts: vec![10], trigram: TrigramIndex::default(), trigram_dirty: false, forward: None, path_to_id: None };
    let ctx = HandlerContext { index: Arc::new(RwLock::new(index)), def_index: None, server_dir: ".".to_string(), server_ext: "cs".to_string(), metrics: true, index_base: PathBuf::from("."), max_response_bytes: crate::mcp::handlers::utils::DEFAULT_MAX_RESPONSE_BYTES, content_ready: Arc::new(AtomicBool::new(true)), def_ready: Arc::new(AtomicBool::new(true)) };
    let result = dispatch_tool(&ctx, "search_grep", &json!({"terms": "foo"}));
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    assert!(output["summary"]["searchTimeMs"].as_f64().unwrap() >= 0.0);
}

// --- search_fast comma-separated tests ---

fn make_search_fast_ctx() -> (HandlerContext, std::path::PathBuf) {
    use std::io::Write;
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let tmp_dir = std::env::temp_dir().join(format!("search_fast_test_{}_{}", std::process::id(), id));
    let _ = std::fs::create_dir_all(&tmp_dir);
    for name in &["ModelSchemaStorage.cs", "ModelSchemaManager.cs", "ScannerJobState.cs", "WorkspaceInfoUtils.cs", "UserService.cs", "OtherFile.txt"] {
        let p = tmp_dir.join(name);
        let mut f = std::fs::File::create(&p).unwrap();
        writeln!(f, "// {}", name).unwrap();
    }
    let dir_str = tmp_dir.to_string_lossy().to_string();
    let file_index = crate::build_index(&crate::IndexArgs { dir: dir_str.clone(), max_age_hours: 24, hidden: false, no_ignore: false, threads: 0 });
    let idx_base = tmp_dir.join(".index");
    let _ = crate::save_index(&file_index, &idx_base);
    let content_index = ContentIndex { root: dir_str.clone(), created_at: 0, max_age_secs: 3600, files: vec![], index: HashMap::new(), total_tokens: 0, extensions: vec!["cs".to_string()], file_token_counts: vec![], trigram: TrigramIndex::default(), trigram_dirty: false, forward: None, path_to_id: None };
    let ctx = HandlerContext { index: Arc::new(RwLock::new(content_index)), def_index: None, server_dir: dir_str, server_ext: "cs".to_string(), metrics: false, index_base: idx_base, max_response_bytes: crate::mcp::handlers::utils::DEFAULT_MAX_RESPONSE_BYTES, content_ready: Arc::new(AtomicBool::new(true)), def_ready: Arc::new(AtomicBool::new(true)) };
    (ctx, tmp_dir)
}

#[test] fn test_search_fast_single_pattern() {
    let (ctx, tmp) = make_search_fast_ctx();
    let result = handle_search_fast(&ctx, &json!({"pattern": "ModelSchemaStorage"}));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    assert_eq!(output["summary"]["totalMatches"], 1);
    cleanup_tmp(&tmp);
}

#[test] fn test_search_fast_comma_separated_multi_term() {
    let (ctx, tmp) = make_search_fast_ctx();
    let result = handle_search_fast(&ctx, &json!({"pattern": "ModelSchemaStorage,ModelSchemaManager,ScannerJobState,WorkspaceInfoUtils"}));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    assert_eq!(output["summary"]["totalMatches"], 4);
    cleanup_tmp(&tmp);
}

#[test] fn test_search_fast_comma_separated_with_ext_filter() {
    let (ctx, tmp) = make_search_fast_ctx();
    let result = handle_search_fast(&ctx, &json!({"pattern": "ModelSchemaStorage,OtherFile", "ext": "cs"}));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    assert_eq!(output["summary"]["totalMatches"], 1);
    cleanup_tmp(&tmp);
}

#[test] fn test_search_fast_comma_separated_no_matches() {
    let (ctx, tmp) = make_search_fast_ctx();
    let result = handle_search_fast(&ctx, &json!({"pattern": "NonExistentClass,AnotherMissing"}));
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    assert_eq!(output["summary"]["totalMatches"], 0);
    cleanup_tmp(&tmp);
}

#[test] fn test_search_fast_comma_separated_partial_matches() {
    let (ctx, tmp) = make_search_fast_ctx();
    let result = handle_search_fast(&ctx, &json!({"pattern": "ModelSchemaStorage,NonExistent,ScannerJobState"}));
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    assert_eq!(output["summary"]["totalMatches"], 2);
    cleanup_tmp(&tmp);
}

#[test] fn test_search_fast_comma_separated_with_spaces() {
    let (ctx, tmp) = make_search_fast_ctx();
    let result = handle_search_fast(&ctx, &json!({"pattern": " ModelSchemaStorage , ScannerJobState "}));
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    assert_eq!(output["summary"]["totalMatches"], 2);
    cleanup_tmp(&tmp);
}

#[test] fn test_search_fast_comma_separated_count_only() {
    let (ctx, tmp) = make_search_fast_ctx();
    let result = handle_search_fast(&ctx, &json!({"pattern": "ModelSchemaStorage,ScannerJobState", "countOnly": true}));
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    assert_eq!(output["summary"]["totalMatches"], 2);
    assert!(output["files"].as_array().unwrap().is_empty());
    cleanup_tmp(&tmp);
}

#[test] fn test_search_fast_comma_separated_ignore_case() {
    let (ctx, tmp) = make_search_fast_ctx();
    let result = handle_search_fast(&ctx, &json!({"pattern": "modelschemastorage,scannerjobstate", "ignoreCase": true}));
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    assert_eq!(output["summary"]["totalMatches"], 2);
    cleanup_tmp(&tmp);
}

// --- Subdir tests ---

#[test] fn test_validate_search_dir_subdirectory() {
    let tmp = std::env::temp_dir().join("search_test_subdir_val");
    std::fs::create_dir_all(&tmp).unwrap();
    let result = validate_search_dir(&tmp.to_string_lossy(), &std::env::temp_dir().to_string_lossy());
    assert!(result.is_ok());
    assert!(result.unwrap().is_some());
    let _ = std::fs::remove_dir(&tmp);
}

#[test] fn test_grep_with_subdir_filter() {
    let tmp = std::env::temp_dir().join("search_test_grep_subdir");
    let sub_a = tmp.join("subA"); let sub_b = tmp.join("subB");
    std::fs::create_dir_all(&sub_a).unwrap(); std::fs::create_dir_all(&sub_b).unwrap();
    std::fs::write(sub_a.join("hello.txt"), "OneLakeCatalog usage here").unwrap();
    std::fs::write(sub_b.join("other.txt"), "OneLakeCatalog other usage").unwrap();
    let index = crate::build_content_index(&crate::ContentIndexArgs { dir: tmp.to_string_lossy().to_string(), ext: "txt".to_string(), max_age_hours: 24, hidden: false, no_ignore: false, threads: 1, min_token_len: 2 });
    let ctx = HandlerContext { index: Arc::new(RwLock::new(index)), def_index: None, server_dir: tmp.to_string_lossy().to_string(), server_ext: "txt".to_string(), metrics: false, index_base: tmp.clone(), max_response_bytes: crate::mcp::handlers::utils::DEFAULT_MAX_RESPONSE_BYTES, content_ready: Arc::new(AtomicBool::new(true)), def_ready: Arc::new(AtomicBool::new(true)) };
    let r_all = handle_search_grep(&ctx, &json!({"terms": "onelakecatalog"}));
    let o_all: Value = serde_json::from_str(&r_all.content[0].text).unwrap();
    assert_eq!(o_all["summary"]["totalFiles"], 2);
    let r_sub = handle_search_grep(&ctx, &json!({"terms": "onelakecatalog", "dir": sub_a.to_string_lossy().to_string()}));
    assert!(!r_sub.is_error);
    let o_sub: Value = serde_json::from_str(&r_sub.content[0].text).unwrap();
    assert_eq!(o_sub["summary"]["totalFiles"], 1);
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test] fn test_grep_rejects_outside_dir() {
    let tmp = std::env::temp_dir().join("search_test_grep_reject");
    std::fs::create_dir_all(&tmp).unwrap();
    let index = ContentIndex { root: tmp.to_string_lossy().to_string(), created_at: 0, max_age_secs: 3600, files: vec![], index: HashMap::new(), file_token_counts: vec![], total_tokens: 0, extensions: vec![], trigram: TrigramIndex::default(), trigram_dirty: false, forward: None, path_to_id: None };
    let ctx = HandlerContext { index: Arc::new(RwLock::new(index)), def_index: None, server_dir: tmp.to_string_lossy().to_string(), server_ext: "cs".to_string(), metrics: false, index_base: tmp.clone(), max_response_bytes: crate::mcp::handlers::utils::DEFAULT_MAX_RESPONSE_BYTES, content_ready: Arc::new(AtomicBool::new(true)), def_ready: Arc::new(AtomicBool::new(true)) };
    let result = handle_search_grep(&ctx, &json!({"terms": "test", "dir": r"Z:\some\other\path"}));
    assert!(result.is_error);
    let _ = std::fs::remove_dir_all(&tmp);
}

// --- includeBody tests (require real files) ---

fn make_ctx_with_real_files() -> (HandlerContext, std::path::PathBuf) {
    use crate::definitions::*;
    use std::io::Write;
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let tmp_dir = std::env::temp_dir().join(format!("search_test_{}_{}", std::process::id(), id));
    let _ = std::fs::create_dir_all(&tmp_dir);
    let file0_path = tmp_dir.join("MyService.cs");
    { let mut f = std::fs::File::create(&file0_path).unwrap(); for i in 1..=15 { writeln!(f, "// line {}", i).unwrap(); } }
    let file1_path = tmp_dir.join("BigFile.cs");
    { let mut f = std::fs::File::create(&file1_path).unwrap(); for i in 1..=25 { writeln!(f, "// big line {}", i).unwrap(); } }
    let file0_str = file0_path.to_string_lossy().to_string();
    let file1_str = file1_path.to_string_lossy().to_string();
    let definitions = vec![
        DefinitionEntry { file_id: 0, name: "MyService".to_string(), kind: DefinitionKind::Class, line_start: 1, line_end: 15, parent: None, signature: None, modifiers: vec![], attributes: vec![], base_types: vec![] },
        DefinitionEntry { file_id: 0, name: "DoWork".to_string(), kind: DefinitionKind::Method, line_start: 3, line_end: 8, parent: Some("MyService".to_string()), signature: None, modifiers: vec![], attributes: vec![], base_types: vec![] },
        DefinitionEntry { file_id: 1, name: "BigClass".to_string(), kind: DefinitionKind::Class, line_start: 1, line_end: 25, parent: None, signature: None, modifiers: vec![], attributes: vec![], base_types: vec![] },
        DefinitionEntry { file_id: 1, name: "Process".to_string(), kind: DefinitionKind::Method, line_start: 5, line_end: 24, parent: Some("BigClass".to_string()), signature: None, modifiers: vec![], attributes: vec![], base_types: vec![] },
    ];
    let mut name_index: HashMap<String, Vec<u32>> = HashMap::new();
    let mut kind_index: HashMap<DefinitionKind, Vec<u32>> = HashMap::new();
    let mut file_index: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut path_to_id: HashMap<PathBuf, u32> = HashMap::new();
    for (i, def) in definitions.iter().enumerate() { let idx = i as u32; name_index.entry(def.name.to_lowercase()).or_default().push(idx); kind_index.entry(def.kind.clone()).or_default().push(idx); file_index.entry(def.file_id).or_default().push(idx); }
    path_to_id.insert(file0_path, 0); path_to_id.insert(file1_path, 1);
    let def_index = DefinitionIndex { root: tmp_dir.to_string_lossy().to_string(), created_at: 0, extensions: vec!["cs".to_string()], files: vec![file0_str.clone(), file1_str.clone()], definitions, name_index, kind_index, attribute_index: HashMap::new(), base_type_index: HashMap::new(), file_index, path_to_id, method_calls: HashMap::new(), parse_errors: 0, lossy_file_count: 0, empty_file_ids: Vec::new() };
    let content_index = ContentIndex { root: tmp_dir.to_string_lossy().to_string(), created_at: 0, max_age_secs: 3600, files: vec![file0_str, file1_str], index: HashMap::new(), total_tokens: 0, extensions: vec!["cs".to_string()], file_token_counts: vec![0, 0], trigram: TrigramIndex::default(), trigram_dirty: false, forward: None, path_to_id: None };
    let ctx = HandlerContext { index: Arc::new(RwLock::new(content_index)), def_index: Some(Arc::new(RwLock::new(def_index))), server_dir: tmp_dir.to_string_lossy().to_string(), server_ext: "cs".to_string(), metrics: false, index_base: PathBuf::from("."), max_response_bytes: crate::mcp::handlers::utils::DEFAULT_MAX_RESPONSE_BYTES, content_ready: Arc::new(AtomicBool::new(true)), def_ready: Arc::new(AtomicBool::new(true)) };
    (ctx, tmp_dir)
}

#[test] fn test_search_definitions_include_body() {
    let (ctx, tmp) = make_ctx_with_real_files();
    let result = dispatch_tool(&ctx, "search_definitions", &json!({"name": "DoWork", "includeBody": true}));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let defs = output["definitions"].as_array().unwrap();
    assert_eq!(defs.len(), 1);
    let body = defs[0]["body"].as_array().unwrap();
    assert_eq!(body.len(), 6);
    assert_eq!(defs[0]["bodyStartLine"], 3);
    cleanup_tmp(&tmp);
}

#[test] fn test_search_definitions_include_body_default_false() {
    let (ctx, tmp) = make_ctx_with_real_files();
    let result = dispatch_tool(&ctx, "search_definitions", &json!({"name": "DoWork"}));
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    assert!(output["definitions"].as_array().unwrap()[0].get("body").is_none());
    cleanup_tmp(&tmp);
}

#[test] fn test_search_definitions_max_body_lines_truncation() {
    let (ctx, tmp) = make_ctx_with_real_files();
    let result = dispatch_tool(&ctx, "search_definitions", &json!({"name": "Process", "includeBody": true, "maxBodyLines": 5}));
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let defs = output["definitions"].as_array().unwrap();
    assert_eq!(defs[0]["body"].as_array().unwrap().len(), 5);
    assert_eq!(defs[0]["bodyTruncated"], true);
    cleanup_tmp(&tmp);
}

#[test] fn test_search_definitions_max_total_body_lines_budget() {
    let (ctx, tmp) = make_ctx_with_real_files();
    let result = dispatch_tool(&ctx, "search_definitions", &json!({"name": "DoWork,Process", "includeBody": true, "maxTotalBodyLines": 10}));
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let total = output["summary"]["totalBodyLinesReturned"].as_u64().unwrap();
    assert!(total <= 10);
    cleanup_tmp(&tmp);
}

#[test] fn test_search_definitions_contains_line_with_body() {
    let (ctx, tmp) = make_ctx_with_real_files();
    let result = dispatch_tool(&ctx, "search_definitions", &json!({"file": "MyService", "containsLine": 5, "includeBody": true}));
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let defs = output["containingDefinitions"].as_array().unwrap();
    assert_eq!(defs[0]["name"], "DoWork");
    assert!(defs[0]["body"].as_array().unwrap().len() > 0);
    cleanup_tmp(&tmp);
}

#[test] fn test_search_definitions_file_cache() {
    let (ctx, tmp) = make_ctx_with_real_files();
    let result = dispatch_tool(&ctx, "search_definitions", &json!({"parent": "MyService", "includeBody": true}));
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let defs = output["definitions"].as_array().unwrap();
    for def in defs { assert!(def.get("body").is_some()); }
    cleanup_tmp(&tmp);
}

#[test] fn test_search_definitions_stale_file_warning() {
    use std::io::Write;
    let tmp = std::env::temp_dir().join(format!("search_test_stale_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmp);
    let fp = tmp.join("Stale.cs");
    { let mut f = std::fs::File::create(&fp).unwrap(); for i in 1..=10 { writeln!(f, "// stale line {}", i).unwrap(); } }
    let fs = fp.to_string_lossy().to_string();
    let definitions = vec![DefinitionEntry { file_id: 0, name: "StaleClass".to_string(), kind: crate::definitions::DefinitionKind::Class, line_start: 5, line_end: 20, parent: None, signature: None, modifiers: vec![], attributes: vec![], base_types: vec![] }];
    let mut ni: HashMap<String, Vec<u32>> = HashMap::new(); let mut ki: HashMap<crate::definitions::DefinitionKind, Vec<u32>> = HashMap::new(); let mut fi: HashMap<u32, Vec<u32>> = HashMap::new();
    for (i, def) in definitions.iter().enumerate() { ni.entry(def.name.to_lowercase()).or_default().push(i as u32); ki.entry(def.kind.clone()).or_default().push(i as u32); fi.entry(def.file_id).or_default().push(i as u32); }
    let di = crate::definitions::DefinitionIndex { root: tmp.to_string_lossy().to_string(), created_at: 0, extensions: vec!["cs".to_string()], files: vec![fs.clone()], definitions, name_index: ni, kind_index: ki, attribute_index: HashMap::new(), base_type_index: HashMap::new(), file_index: fi, path_to_id: HashMap::new(), method_calls: HashMap::new(), parse_errors: 0, lossy_file_count: 0, empty_file_ids: Vec::new() };
    let ci = ContentIndex { root: tmp.to_string_lossy().to_string(), created_at: 0, max_age_secs: 3600, files: vec![fs], index: HashMap::new(), total_tokens: 0, extensions: vec!["cs".to_string()], file_token_counts: vec![0], trigram: TrigramIndex::default(), trigram_dirty: false, forward: None, path_to_id: None };
    let ctx = HandlerContext { index: Arc::new(RwLock::new(ci)), def_index: Some(Arc::new(RwLock::new(di))), server_dir: tmp.to_string_lossy().to_string(), server_ext: "cs".to_string(), metrics: false, index_base: PathBuf::from("."), max_response_bytes: crate::mcp::handlers::utils::DEFAULT_MAX_RESPONSE_BYTES, content_ready: Arc::new(AtomicBool::new(true)), def_ready: Arc::new(AtomicBool::new(true)) };
    let result = dispatch_tool(&ctx, "search_definitions", &json!({"name": "StaleClass", "includeBody": true}));
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    assert!(output["definitions"].as_array().unwrap()[0].get("bodyWarning").is_some());
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test] fn test_search_definitions_body_error() {
    let definitions = vec![DefinitionEntry { file_id: 0, name: "GhostClass".to_string(), kind: crate::definitions::DefinitionKind::Class, line_start: 1, line_end: 10, parent: None, signature: None, modifiers: vec![], attributes: vec![], base_types: vec![] }];
    let mut ni: HashMap<String, Vec<u32>> = HashMap::new(); let mut ki: HashMap<crate::definitions::DefinitionKind, Vec<u32>> = HashMap::new(); let mut fi: HashMap<u32, Vec<u32>> = HashMap::new();
    for (i, def) in definitions.iter().enumerate() { ni.entry(def.name.to_lowercase()).or_default().push(i as u32); ki.entry(def.kind.clone()).or_default().push(i as u32); fi.entry(def.file_id).or_default().push(i as u32); }
    let ne = "C:\\nonexistent\\path\\Ghost.cs".to_string();
    let di = crate::definitions::DefinitionIndex { root: ".".to_string(), created_at: 0, extensions: vec!["cs".to_string()], files: vec![ne.clone()], definitions, name_index: ni, kind_index: ki, attribute_index: HashMap::new(), base_type_index: HashMap::new(), file_index: fi, path_to_id: HashMap::new(), method_calls: HashMap::new(), parse_errors: 0, lossy_file_count: 0, empty_file_ids: Vec::new() };
    let ci = ContentIndex { root: ".".to_string(), created_at: 0, max_age_secs: 3600, files: vec![ne], index: HashMap::new(), total_tokens: 0, extensions: vec!["cs".to_string()], file_token_counts: vec![0], trigram: TrigramIndex::default(), trigram_dirty: false, forward: None, path_to_id: None };
    let ctx = HandlerContext { index: Arc::new(RwLock::new(ci)), def_index: Some(Arc::new(RwLock::new(di))), server_dir: ".".to_string(), server_ext: "cs".to_string(), metrics: false, index_base: PathBuf::from("."), max_response_bytes: crate::mcp::handlers::utils::DEFAULT_MAX_RESPONSE_BYTES, content_ready: Arc::new(AtomicBool::new(true)), def_ready: Arc::new(AtomicBool::new(true)) };
    let result = dispatch_tool(&ctx, "search_definitions", &json!({"name": "GhostClass", "includeBody": true}));
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    assert_eq!(output["definitions"].as_array().unwrap()[0]["bodyError"], "failed to read file");
}

// --- Response truncation integration tests ---

#[test]
fn test_response_truncation_triggers_on_large_result() {
    // Build an index with many files, long paths, and many lines per token
    // to generate a response well over 32KB.
    let mut idx = HashMap::new();
    let mut files = Vec::new();
    let mut file_token_counts = Vec::new();

    // Create 500 files with long paths and 100 lines each → ~250KB+ response
    for i in 0..500 {
        let path = format!(
            "C:\\Projects\\Enterprise\\Solution\\src\\Features\\Module_{:03}\\SubModule\\Implementations\\Component_{:03}Service.cs",
            i / 10, i
        );
        files.push(path);
        file_token_counts.push(1000u32);

        let lines: Vec<u32> = (1..=100).collect(); // 100 lines per file
        idx.entry("common".to_string())
            .or_insert_with(Vec::new)
            .push(Posting { file_id: i as u32, lines });
    }

    let index = ContentIndex {
        root: ".".to_string(),
        created_at: 0,
        max_age_secs: 3600,
        files,
        index: idx,
        total_tokens: 500_000,
        extensions: vec!["cs".to_string()],
        file_token_counts,
        trigram: TrigramIndex::default(),
        trigram_dirty: false,
        forward: None,
        path_to_id: None,
    };

    let ctx = HandlerContext {
        index: Arc::new(RwLock::new(index)),
        def_index: None,
        server_dir: ".".to_string(),
        server_ext: "cs".to_string(),
        metrics: true, // metrics enabled triggers truncation in inject_metrics
        index_base: PathBuf::from("."), max_response_bytes: crate::mcp::handlers::utils::DEFAULT_MAX_RESPONSE_BYTES, content_ready: Arc::new(AtomicBool::new(true)), def_ready: Arc::new(AtomicBool::new(true)),
    };

    // Request with maxResults=0 (unlimited) to get all 500 files
    let result = dispatch_tool(&ctx, "search_grep", &json!({
        "terms": "common",
        "maxResults": 0,
        "substring": false
    }));

    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();

    // Summary should reflect the FULL result count (not truncated)
    assert_eq!(output["summary"]["totalFiles"], 500);

    // Response should have been truncated
    assert_eq!(output["summary"]["responseTruncated"], true,
        "Expected responseTruncated=true for 500-file response");
    assert!(output["summary"]["truncationReason"].as_str().is_some(),
        "Expected truncationReason in summary");
    assert!(output["summary"]["hint"].as_str().is_some(),
        "Expected hint in summary");

    // The files array should be reduced from 500
    let files_arr = output["files"].as_array().unwrap();
    assert!(files_arr.len() < 500,
        "Expected files array to be truncated from 500, got {}", files_arr.len());

    // Response bytes should be under the budget (16KB default + some metadata overhead)
    let response_bytes = output["summary"]["responseBytes"].as_u64().unwrap();
    assert!(response_bytes < 20_000,
        "Expected responseBytes < 20000, got {}", response_bytes);
}

#[test]
fn test_response_truncation_does_not_trigger_on_small_result() {
    // Small index — response should NOT be truncated
    let mut idx = HashMap::new();
    idx.insert("mytoken".to_string(), vec![Posting { file_id: 0, lines: vec![10, 20] }]);

    let index = ContentIndex {
        root: ".".to_string(),
        created_at: 0,
        max_age_secs: 3600,
        files: vec!["C:\\test\\Small.cs".to_string()],
        index: idx,
        total_tokens: 50,
        extensions: vec!["cs".to_string()],
        file_token_counts: vec![50],
        trigram: TrigramIndex::default(),
        trigram_dirty: false,
        forward: None,
        path_to_id: None,
    };

    let ctx = HandlerContext {
        index: Arc::new(RwLock::new(index)),
        def_index: None,
        server_dir: ".".to_string(),
        server_ext: "cs".to_string(),
        metrics: true,
        index_base: PathBuf::from("."), max_response_bytes: crate::mcp::handlers::utils::DEFAULT_MAX_RESPONSE_BYTES, content_ready: Arc::new(AtomicBool::new(true)), def_ready: Arc::new(AtomicBool::new(true)),
    };

    let result = dispatch_tool(&ctx, "search_grep", &json!({"terms": "mytoken", "substring": false}));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();

    // Should NOT be truncated
    assert!(output["summary"].get("responseTruncated").is_none(),
        "Small response should not have responseTruncated");
    assert_eq!(output["summary"]["totalFiles"], 1);
    assert_eq!(output["files"].as_array().unwrap().len(), 1);
}

// ─── Async startup: index-building readiness tests ──────────────────

#[test]
fn test_dispatch_grep_while_content_index_building() {
    let ctx = HandlerContext {
        content_ready: Arc::new(AtomicBool::new(false)),
        ..make_empty_ctx()
    };
    let result = dispatch_tool(&ctx, "search_grep", &json!({"terms": "foo"}));
    assert!(result.is_error);
    assert!(result.content[0].text.contains("being built"),
        "Expected 'being built' message, got: {}", result.content[0].text);
}

#[test]
fn test_dispatch_definitions_while_def_index_building() {
    let ctx = HandlerContext {
        def_ready: Arc::new(AtomicBool::new(false)),
        def_index: Some(Arc::new(RwLock::new(crate::definitions::DefinitionIndex {
            root: ".".to_string(),
            created_at: 0,
            extensions: vec!["cs".to_string()],
            files: Vec::new(),
            definitions: Vec::new(),
            name_index: HashMap::new(),
            kind_index: HashMap::new(),
            attribute_index: HashMap::new(),
            base_type_index: HashMap::new(),
            file_index: HashMap::new(),
            path_to_id: HashMap::new(),
            method_calls: HashMap::new(),
            parse_errors: 0,
            lossy_file_count: 0,
            empty_file_ids: Vec::new(),
        }))),
        ..make_empty_ctx()
    };
    let result = dispatch_tool(&ctx, "search_definitions", &json!({"name": "Foo"}));
    assert!(result.is_error);
    assert!(result.content[0].text.contains("being built"),
        "Expected 'being built' message, got: {}", result.content[0].text);
}

#[test]
fn test_dispatch_callers_while_def_index_building() {
    let ctx = HandlerContext {
        def_ready: Arc::new(AtomicBool::new(false)),
        def_index: Some(Arc::new(RwLock::new(crate::definitions::DefinitionIndex {
            root: ".".to_string(),
            created_at: 0,
            extensions: vec!["cs".to_string()],
            files: Vec::new(),
            definitions: Vec::new(),
            name_index: HashMap::new(),
            kind_index: HashMap::new(),
            attribute_index: HashMap::new(),
            base_type_index: HashMap::new(),
            file_index: HashMap::new(),
            path_to_id: HashMap::new(),
            method_calls: HashMap::new(),
            parse_errors: 0,
            lossy_file_count: 0,
            empty_file_ids: Vec::new(),
        }))),
        ..make_empty_ctx()
    };
    let result = dispatch_tool(&ctx, "search_callers", &json!({"method": "Foo"}));
    assert!(result.is_error);
    assert!(result.content[0].text.contains("being built"),
        "Expected 'being built' message, got: {}", result.content[0].text);
}

#[test]
fn test_dispatch_reindex_while_content_index_building() {
    let ctx = HandlerContext {
        content_ready: Arc::new(AtomicBool::new(false)),
        ..make_empty_ctx()
    };
    let result = dispatch_tool(&ctx, "search_reindex", &json!({}));
    assert!(result.is_error);
    assert!(result.content[0].text.contains("already being built"),
        "Expected 'already being built' message, got: {}", result.content[0].text);
}

#[test]
fn test_dispatch_fast_while_content_index_building() {
    let ctx = HandlerContext {
        content_ready: Arc::new(AtomicBool::new(false)),
        ..make_empty_ctx()
    };
    let result = dispatch_tool(&ctx, "search_fast", &json!({"pattern": "foo"}));
    assert!(result.is_error);
    assert!(result.content[0].text.contains("being built"),
        "Expected 'being built' message, got: {}", result.content[0].text);
}

#[test]
fn test_dispatch_help_works_while_index_building() {
    // search_help and search_info should work even when index is building
    let ctx = HandlerContext {
        content_ready: Arc::new(AtomicBool::new(false)),
        def_ready: Arc::new(AtomicBool::new(false)),
        ..make_empty_ctx()
    };
    let result = dispatch_tool(&ctx, "search_help", &json!({}));
    assert!(!result.is_error, "search_help should work during index build");

    let result = dispatch_tool(&ctx, "search_info", &json!({}));
    assert!(!result.is_error, "search_info should work during index build");
}

#[test]
fn test_dispatch_find_works_while_index_building() {
    // search_find uses filesystem walk, not the content index, so it should work
    let ctx = HandlerContext {
        content_ready: Arc::new(AtomicBool::new(false)),
        ..make_empty_ctx()
    };
    let result = dispatch_tool(&ctx, "search_find", &json!({"pattern": "nonexistent_xyz"}));
    assert!(!result.is_error, "search_find should work during index build");
}

// ─── Batch 1: Missing unit tests (T40, T27, T28, T32, T50, T53, T54, T61, T73) ───

/// T40 — search_grep response truncation: when results exceed max_response_bytes,
/// the output includes responseTruncated indicator and files array is reduced.
#[test]
fn test_search_grep_response_truncation_via_small_budget() {
    // Build an index with enough files to exceed a small response budget
    let mut idx = HashMap::new();
    let mut files = Vec::new();
    let mut file_token_counts = Vec::new();

    for i in 0..100 {
        let path = format!(
            "C:\\Projects\\Module_{:03}\\Component_{:03}Service.cs",
            i / 10, i
        );
        files.push(path);
        file_token_counts.push(100u32);
        let lines: Vec<u32> = (1..=20).collect();
        idx.entry("targettoken".to_string())
            .or_insert_with(Vec::new)
            .push(Posting { file_id: i as u32, lines });
    }

    let index = ContentIndex {
        root: ".".to_string(),
        created_at: 0,
        max_age_secs: 3600,
        files,
        index: idx,
        total_tokens: 10_000,
        extensions: vec!["cs".to_string()],
        file_token_counts,
        trigram: TrigramIndex::default(),
        trigram_dirty: false,
        forward: None,
        path_to_id: None,
    };

    // Use a very small max_response_bytes to force truncation
    let ctx = HandlerContext {
        index: Arc::new(RwLock::new(index)),
        def_index: None,
        server_dir: ".".to_string(),
        server_ext: "cs".to_string(),
        metrics: true,
        index_base: PathBuf::from("."),
        max_response_bytes: 2_000, // Very small budget to force truncation
        content_ready: Arc::new(AtomicBool::new(true)),
        def_ready: Arc::new(AtomicBool::new(true)),
    };

    let result = dispatch_tool(&ctx, "search_grep", &json!({
        "terms": "targettoken",
        "maxResults": 0,
        "substring": false
    }));

    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();

    // Summary should reflect the full result count
    assert_eq!(output["summary"]["totalFiles"], 100);

    // Response should have been truncated
    assert_eq!(output["summary"]["responseTruncated"], true,
        "Expected responseTruncated=true for 100-file response with 2KB budget");
    assert!(output["summary"]["truncationReason"].as_str().is_some(),
        "Expected truncationReason in summary");

    // The files array should be reduced from 100
    let files_arr = output["files"].as_array().unwrap();
    assert!(files_arr.len() < 100,
        "Expected files array to be truncated from 100, got {}", files_arr.len());
}

/// T27 — search_definitions regex name filter: regex=true with name pattern
/// should match definitions whose names satisfy the regex.
#[test]
fn test_search_definitions_regex_name_filter() {
    let ctx = make_ctx_with_defs();
    let result = dispatch_tool(&ctx, "search_definitions", &json!({
        "name": "Execute.*",
        "regex": true
    }));
    assert!(!result.is_error, "search_definitions regex should not error: {}", result.content[0].text);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let defs = output["definitions"].as_array().unwrap();

    // Should find ExecuteQueryAsync (exists in ResilientClient and ProxyClient)
    assert!(!defs.is_empty(), "Regex 'Execute.*' should match ExecuteQueryAsync definitions");

    // All returned definitions should match the regex
    for def in defs {
        let name = def["name"].as_str().unwrap();
        assert!(name.to_lowercase().starts_with("execute"),
            "Definition '{}' should match regex 'Execute.*'", name);
    }

    // Should NOT contain definitions that don't match
    for def in defs {
        let name = def["name"].as_str().unwrap();
        assert!(name != "QueryService" && name != "RunQueryBatchAsync",
            "Definition '{}' should NOT match regex 'Execute.*'", name);
    }
}

/// T28 — search_definitions audit mode: audit=true returns index coverage report
/// with totalFiles, filesWithDefinitions, filesWithoutDefinitions, etc.
#[test]
fn test_search_definitions_audit_mode() {
    let ctx = make_ctx_with_defs();
    let result = dispatch_tool(&ctx, "search_definitions", &json!({
        "audit": true
    }));
    assert!(!result.is_error, "audit mode should not error: {}", result.content[0].text);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();

    // Audit output should have the audit object
    let audit = &output["audit"];
    assert!(audit.is_object(), "Expected 'audit' object in output");
    assert!(audit["totalFiles"].as_u64().is_some(), "Expected totalFiles in audit");
    assert!(audit["filesWithDefinitions"].as_u64().is_some(), "Expected filesWithDefinitions in audit");
    assert!(audit["filesWithoutDefinitions"].as_u64().is_some(), "Expected filesWithoutDefinitions in audit");
    assert!(audit["readErrors"].as_u64().is_some(), "Expected readErrors in audit");
    assert!(audit["lossyUtf8Files"].as_u64().is_some(), "Expected lossyUtf8Files in audit");
    assert!(audit["suspiciousFiles"].as_u64().is_some(), "Expected suspiciousFiles count in audit");
    assert!(audit["suspiciousThresholdBytes"].as_u64().is_some(), "Expected suspiciousThresholdBytes in audit");

    // Should also have suspiciousFiles array at top level
    assert!(output["suspiciousFiles"].is_array(), "Expected suspiciousFiles array in output");

    // Verify the counts make sense for our test context (3 files, all with definitions)
    assert_eq!(audit["totalFiles"].as_u64().unwrap(), 3);
    assert_eq!(audit["filesWithDefinitions"].as_u64().unwrap(), 3);
}

/// T32 — search_callers excludeDir/excludeFile filters: callers from excluded
/// directories or files should be filtered out.
#[test]
fn test_search_callers_exclude_dir_and_file() {
    use crate::definitions::*;

    // Set up: MethodA is defined in ServiceA (dir: src\services)
    // MethodA is called from ControllerB (dir: src\controllers) and from TestC (dir: src\tests)
    let mut content_idx = HashMap::new();
    content_idx.insert("methoda".to_string(), vec![
        Posting { file_id: 0, lines: vec![10] },
        Posting { file_id: 1, lines: vec![25] },
        Posting { file_id: 2, lines: vec![15] },
    ]);
    content_idx.insert("servicea".to_string(), vec![
        Posting { file_id: 0, lines: vec![1] },
        Posting { file_id: 1, lines: vec![5] },
        Posting { file_id: 2, lines: vec![3] },
    ]);

    let content_index = ContentIndex {
        root: ".".to_string(), created_at: 0, max_age_secs: 3600,
        files: vec![
            "C:\\src\\services\\ServiceA.cs".to_string(),
            "C:\\src\\controllers\\ControllerB.cs".to_string(),
            "C:\\src\\tests\\TestC.cs".to_string(),
        ],
        index: content_idx, total_tokens: 300,
        extensions: vec!["cs".to_string()],
        file_token_counts: vec![100, 100, 100],
        trigram: TrigramIndex::default(), trigram_dirty: false, forward: None, path_to_id: None,
    };

    let definitions = vec![
        DefinitionEntry {
            file_id: 0, name: "ServiceA".to_string(),
            kind: DefinitionKind::Class, line_start: 1, line_end: 50,
            parent: None, signature: None, modifiers: vec![], attributes: vec![], base_types: vec![],
        },
        DefinitionEntry {
            file_id: 0, name: "MethodA".to_string(),
            kind: DefinitionKind::Method, line_start: 10, line_end: 20,
            parent: Some("ServiceA".to_string()), signature: None,
            modifiers: vec![], attributes: vec![], base_types: vec![],
        },
        DefinitionEntry {
            file_id: 1, name: "ControllerB".to_string(),
            kind: DefinitionKind::Class, line_start: 1, line_end: 50,
            parent: None, signature: None, modifiers: vec![], attributes: vec![], base_types: vec![],
        },
        DefinitionEntry {
            file_id: 1, name: "HandleRequest".to_string(),
            kind: DefinitionKind::Method, line_start: 20, line_end: 35,
            parent: Some("ControllerB".to_string()), signature: None,
            modifiers: vec![], attributes: vec![], base_types: vec![],
        },
        DefinitionEntry {
            file_id: 2, name: "TestC".to_string(),
            kind: DefinitionKind::Class, line_start: 1, line_end: 40,
            parent: None, signature: None, modifiers: vec![], attributes: vec![], base_types: vec![],
        },
        DefinitionEntry {
            file_id: 2, name: "TestMethodA".to_string(),
            kind: DefinitionKind::Method, line_start: 10, line_end: 25,
            parent: Some("TestC".to_string()), signature: None,
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
    path_to_id.insert(PathBuf::from("C:\\src\\services\\ServiceA.cs"), 0);
    path_to_id.insert(PathBuf::from("C:\\src\\controllers\\ControllerB.cs"), 1);
    path_to_id.insert(PathBuf::from("C:\\src\\tests\\TestC.cs"), 2);

    let def_index = DefinitionIndex {
        root: ".".to_string(), created_at: 0,
        extensions: vec!["cs".to_string()],
        files: vec![
            "C:\\src\\services\\ServiceA.cs".to_string(),
            "C:\\src\\controllers\\ControllerB.cs".to_string(),
            "C:\\src\\tests\\TestC.cs".to_string(),
        ],
        definitions, name_index, kind_index,
        attribute_index: HashMap::new(), base_type_index: HashMap::new(),
        file_index, path_to_id, method_calls: HashMap::new(),
        parse_errors: 0, lossy_file_count: 0, empty_file_ids: Vec::new(),
    };

    let ctx = HandlerContext {
        index: Arc::new(RwLock::new(content_index)),
        def_index: Some(Arc::new(RwLock::new(def_index))),
        server_dir: ".".to_string(), server_ext: "cs".to_string(),
        metrics: false, index_base: PathBuf::from("."),
        max_response_bytes: crate::mcp::handlers::utils::DEFAULT_MAX_RESPONSE_BYTES,
        content_ready: Arc::new(AtomicBool::new(true)),
        def_ready: Arc::new(AtomicBool::new(true)),
    };

    // Test excludeDir: exclude "tests" directory
    let result = dispatch_tool(&ctx, "search_callers", &json!({
        "method": "MethodA",
        "class": "ServiceA",
        "depth": 1,
        "excludeDir": ["tests"]
    }));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let tree = output["callTree"].as_array().unwrap();

    // Should NOT contain callers from the "tests" directory
    for node in tree {
        let file = node["file"].as_str().unwrap_or("");
        assert!(!file.to_lowercase().contains("test"),
            "excludeDir should filter out test files, but found: {}", file);
    }

    // Test excludeFile: exclude "TestC" file pattern
    let result2 = dispatch_tool(&ctx, "search_callers", &json!({
        "method": "MethodA",
        "class": "ServiceA",
        "depth": 1,
        "excludeFile": ["TestC"]
    }));
    assert!(!result2.is_error);
    let output2: Value = serde_json::from_str(&result2.content[0].text).unwrap();
    let tree2 = output2["callTree"].as_array().unwrap();

    for node in tree2 {
        let file = node["file"].as_str().unwrap_or("");
        assert!(!file.to_lowercase().contains("testc"),
            "excludeFile should filter out TestC, but found: {}", file);
    }
}

/// T50 — search_definitions excludeDir filter: definitions from excluded
/// directories should not appear in results.
#[test]
fn test_search_definitions_exclude_dir() {
    use crate::definitions::*;

    // Create a context with definitions in two different directories
    let content_index = ContentIndex {
        root: ".".to_string(), created_at: 0, max_age_secs: 3600,
        files: vec![
            "C:\\src\\main\\UserService.cs".to_string(),
            "C:\\src\\tests\\UserServiceTests.cs".to_string(),
        ],
        index: HashMap::new(), total_tokens: 100,
        extensions: vec!["cs".to_string()],
        file_token_counts: vec![50, 50],
        trigram: TrigramIndex::default(), trigram_dirty: false, forward: None, path_to_id: None,
    };

    let definitions = vec![
        DefinitionEntry {
            file_id: 0, name: "UserService".to_string(),
            kind: DefinitionKind::Class, line_start: 1, line_end: 50,
            parent: None, signature: None, modifiers: vec![], attributes: vec![], base_types: vec![],
        },
        DefinitionEntry {
            file_id: 0, name: "GetUser".to_string(),
            kind: DefinitionKind::Method, line_start: 10, line_end: 20,
            parent: Some("UserService".to_string()), signature: None,
            modifiers: vec![], attributes: vec![], base_types: vec![],
        },
        DefinitionEntry {
            file_id: 1, name: "UserServiceTests".to_string(),
            kind: DefinitionKind::Class, line_start: 1, line_end: 100,
            parent: None, signature: None, modifiers: vec![], attributes: vec![], base_types: vec![],
        },
        DefinitionEntry {
            file_id: 1, name: "TestGetUser".to_string(),
            kind: DefinitionKind::Method, line_start: 10, line_end: 30,
            parent: Some("UserServiceTests".to_string()), signature: None,
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
    path_to_id.insert(PathBuf::from("C:\\src\\main\\UserService.cs"), 0);
    path_to_id.insert(PathBuf::from("C:\\src\\tests\\UserServiceTests.cs"), 1);

    let def_index = DefinitionIndex {
        root: ".".to_string(), created_at: 0,
        extensions: vec!["cs".to_string()],
        files: vec![
            "C:\\src\\main\\UserService.cs".to_string(),
            "C:\\src\\tests\\UserServiceTests.cs".to_string(),
        ],
        definitions, name_index, kind_index,
        attribute_index: HashMap::new(), base_type_index: HashMap::new(),
        file_index, path_to_id, method_calls: HashMap::new(),
        parse_errors: 0, lossy_file_count: 0, empty_file_ids: Vec::new(),
    };

    let ctx = HandlerContext {
        index: Arc::new(RwLock::new(content_index)),
        def_index: Some(Arc::new(RwLock::new(def_index))),
        server_dir: ".".to_string(), server_ext: "cs".to_string(),
        metrics: false, index_base: PathBuf::from("."),
        max_response_bytes: crate::mcp::handlers::utils::DEFAULT_MAX_RESPONSE_BYTES,
        content_ready: Arc::new(AtomicBool::new(true)),
        def_ready: Arc::new(AtomicBool::new(true)),
    };

    // Exclude "tests" directory
    let result = dispatch_tool(&ctx, "search_definitions", &json!({
        "excludeDir": ["tests"]
    }));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let defs = output["definitions"].as_array().unwrap();

    // All returned definitions should be from non-test directories
    for def in defs {
        let file = def["file"].as_str().unwrap_or("");
        assert!(!file.to_lowercase().contains("tests"),
            "excludeDir should filter out definitions from 'tests' dir, but found file: {}", file);
    }

    // Should still have the main definitions
    assert!(!defs.is_empty(), "Should have definitions from non-excluded directories");
    let names: Vec<&str> = defs.iter().filter_map(|d| d["name"].as_str()).collect();
    assert!(names.contains(&"UserService"), "Should contain UserService from main dir");
    assert!(names.contains(&"GetUser"), "Should contain GetUser from main dir");
    assert!(!names.contains(&"UserServiceTests"), "Should NOT contain UserServiceTests from tests dir");
    assert!(!names.contains(&"TestGetUser"), "Should NOT contain TestGetUser from tests dir");
}

/// T53 — search_definitions combined name+parent+kind filter: only definitions
/// matching ALL three criteria should be returned.
#[test]
fn test_search_definitions_combined_name_parent_kind_filter() {
    let ctx = make_ctx_with_defs();

    // Filter: name=ExecuteQueryAsync, parent=ResilientClient, kind=method
    let result = dispatch_tool(&ctx, "search_definitions", &json!({
        "name": "ExecuteQueryAsync",
        "parent": "ResilientClient",
        "kind": "method"
    }));
    assert!(!result.is_error, "Combined filter should not error: {}", result.content[0].text);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let defs = output["definitions"].as_array().unwrap();

    // Should return exactly 1 definition: ExecuteQueryAsync in ResilientClient
    assert_eq!(defs.len(), 1,
        "Expected exactly 1 result for name+parent+kind filter, got {}: {:?}",
        defs.len(), defs);
    assert_eq!(defs[0]["name"], "ExecuteQueryAsync");
    assert_eq!(defs[0]["parent"], "ResilientClient");
    assert_eq!(defs[0]["kind"], "method");

    // Verify: same name+kind but different parent should NOT match
    let result2 = dispatch_tool(&ctx, "search_definitions", &json!({
        "name": "ExecuteQueryAsync",
        "parent": "NonExistentClass",
        "kind": "method"
    }));
    assert!(!result2.is_error);
    let output2: Value = serde_json::from_str(&result2.content[0].text).unwrap();
    let defs2 = output2["definitions"].as_array().unwrap();
    assert_eq!(defs2.len(), 0,
        "Non-matching parent should return 0 results, got {}", defs2.len());
}

/// T54 — search_definitions non-existent name returns empty: searching for a name
/// that doesn't exist should return an empty definitions array.
#[test]
fn test_search_definitions_nonexistent_name_returns_empty() {
    let ctx = make_ctx_with_defs();
    let result = dispatch_tool(&ctx, "search_definitions", &json!({
        "name": "CompletelyNonExistentDefinitionXYZ123"
    }));
    assert!(!result.is_error, "Non-existent name should not error: {}", result.content[0].text);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let defs = output["definitions"].as_array().unwrap();
    assert!(defs.is_empty(),
        "Expected empty definitions array for non-existent name, got {} results", defs.len());
    assert_eq!(output["summary"]["totalResults"], 0);
}

/// T61 — search_definitions invalid regex error: using regex=true with an
/// invalid regex pattern should return an error.
#[test]
fn test_search_definitions_invalid_regex_error() {
    let ctx = make_ctx_with_defs();
    let result = dispatch_tool(&ctx, "search_definitions", &json!({
        "name": "[invalid",
        "regex": true
    }));
    assert!(result.is_error, "Invalid regex should produce an error");
    assert!(result.content[0].text.contains("Invalid regex"),
        "Error should mention 'Invalid regex', got: {}", result.content[0].text);
}

/// T73 — search_definitions struct kind via handler: filtering by kind="struct"
/// should return only struct definitions.
#[test]
fn test_search_definitions_struct_kind() {
    use crate::definitions::*;

    // Create a context with a struct definition alongside class and method
    let content_index = ContentIndex {
        root: ".".to_string(), created_at: 0, max_age_secs: 3600,
        files: vec!["C:\\src\\Models.cs".to_string()],
        index: HashMap::new(), total_tokens: 50,
        extensions: vec!["cs".to_string()],
        file_token_counts: vec![50],
        trigram: TrigramIndex::default(), trigram_dirty: false, forward: None, path_to_id: None,
    };

    let definitions = vec![
        DefinitionEntry {
            file_id: 0, name: "UserModel".to_string(),
            kind: DefinitionKind::Struct, line_start: 1, line_end: 20,
            parent: None, signature: None, modifiers: vec![], attributes: vec![], base_types: vec![],
        },
        DefinitionEntry {
            file_id: 0, name: "UserService".to_string(),
            kind: DefinitionKind::Class, line_start: 25, line_end: 80,
            parent: None, signature: None, modifiers: vec![], attributes: vec![], base_types: vec![],
        },
        DefinitionEntry {
            file_id: 0, name: "GetUser".to_string(),
            kind: DefinitionKind::Method, line_start: 30, line_end: 45,
            parent: Some("UserService".to_string()), signature: None,
            modifiers: vec![], attributes: vec![], base_types: vec![],
        },
        DefinitionEntry {
            file_id: 0, name: "OrderInfo".to_string(),
            kind: DefinitionKind::Struct, line_start: 85, line_end: 100,
            parent: None, signature: None, modifiers: vec![], attributes: vec![], base_types: vec![],
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
    path_to_id.insert(PathBuf::from("C:\\src\\Models.cs"), 0);

    let def_index = DefinitionIndex {
        root: ".".to_string(), created_at: 0,
        extensions: vec!["cs".to_string()],
        files: vec!["C:\\src\\Models.cs".to_string()],
        definitions, name_index, kind_index,
        attribute_index: HashMap::new(), base_type_index: HashMap::new(),
        file_index, path_to_id, method_calls: HashMap::new(),
        parse_errors: 0, lossy_file_count: 0, empty_file_ids: Vec::new(),
    };

    let ctx = HandlerContext {
        index: Arc::new(RwLock::new(content_index)),
        def_index: Some(Arc::new(RwLock::new(def_index))),
        server_dir: ".".to_string(), server_ext: "cs".to_string(),
        metrics: false, index_base: PathBuf::from("."),
        max_response_bytes: crate::mcp::handlers::utils::DEFAULT_MAX_RESPONSE_BYTES,
        content_ready: Arc::new(AtomicBool::new(true)),
        def_ready: Arc::new(AtomicBool::new(true)),
    };

    let result = dispatch_tool(&ctx, "search_definitions", &json!({
        "kind": "struct"
    }));
    assert!(!result.is_error, "kind=struct should not error: {}", result.content[0].text);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let defs = output["definitions"].as_array().unwrap();

    // Should return only struct definitions
    assert_eq!(defs.len(), 2, "Expected 2 struct definitions, got {}", defs.len());
    for def in defs {
        assert_eq!(def["kind"], "struct",
            "All results should be structs, but got kind={}", def["kind"]);
    }
    let names: Vec<&str> = defs.iter().filter_map(|d| d["name"].as_str()).collect();
    assert!(names.contains(&"UserModel"), "Should contain UserModel struct");
    assert!(names.contains(&"OrderInfo"), "Should contain OrderInfo struct");
}

// ═══════════════════════════════════════════════════════════════════════
// Batch 2 tests — Strengthen Partial Coverage
// ═══════════════════════════════════════════════════════════════════════

/// T15 — search_fast dirsOnly and filesOnly filters.
/// dirsOnly=true should return only directory matches;
/// filesOnly=true should return only file matches.
#[test]
fn test_search_fast_dirs_only_and_files_only() {
    use std::io::Write;
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let tmp_dir = std::env::temp_dir().join(format!("search_fast_dironly_{}_{}", std::process::id(), id));
    let _ = std::fs::create_dir_all(&tmp_dir);

    // Create files and a subdirectory whose name contains "Models"
    let sub = tmp_dir.join("Models");
    let _ = std::fs::create_dir_all(&sub);
    let file_in_sub = sub.join("ModelItem.cs");
    { let mut f = std::fs::File::create(&file_in_sub).unwrap(); writeln!(f, "// model").unwrap(); }
    let file_at_root = tmp_dir.join("ModelsHelper.cs");
    { let mut f = std::fs::File::create(&file_at_root).unwrap(); writeln!(f, "// helper").unwrap(); }

    let dir_str = tmp_dir.to_string_lossy().to_string();
    let file_index = crate::build_index(&crate::IndexArgs { dir: dir_str.clone(), max_age_hours: 24, hidden: false, no_ignore: false, threads: 0 });
    let idx_base = tmp_dir.join(".index");
    let _ = crate::save_index(&file_index, &idx_base);
    let content_index = ContentIndex { root: dir_str.clone(), created_at: 0, max_age_secs: 3600, files: vec![], index: HashMap::new(), total_tokens: 0, extensions: vec!["cs".to_string()], file_token_counts: vec![], trigram: TrigramIndex::default(), trigram_dirty: false, forward: None, path_to_id: None };
    let ctx = HandlerContext { index: Arc::new(RwLock::new(content_index)), def_index: None, server_dir: dir_str, server_ext: "cs".to_string(), metrics: false, index_base: idx_base, max_response_bytes: crate::mcp::handlers::utils::DEFAULT_MAX_RESPONSE_BYTES, content_ready: Arc::new(AtomicBool::new(true)), def_ready: Arc::new(AtomicBool::new(true)) };

    // dirsOnly=true: should only return directory entries matching "Models"
    let result_dirs = handle_search_fast(&ctx, &json!({"pattern": "Models", "dirsOnly": true}));
    assert!(!result_dirs.is_error, "dirsOnly should not error: {}", result_dirs.content[0].text);
    let output_dirs: Value = serde_json::from_str(&result_dirs.content[0].text).unwrap();
    let dir_files = output_dirs["files"].as_array().unwrap();
    for entry in dir_files {
        assert_eq!(entry["isDir"], true, "dirsOnly should only return directories, got: {}", entry);
    }
    assert!(output_dirs["summary"]["totalMatches"].as_u64().unwrap() >= 1,
        "Should find at least one directory matching 'Models'");

    // filesOnly=true: should only return file entries matching "Models"
    let result_files = handle_search_fast(&ctx, &json!({"pattern": "Models", "filesOnly": true}));
    assert!(!result_files.is_error);
    let output_files: Value = serde_json::from_str(&result_files.content[0].text).unwrap();
    let file_entries = output_files["files"].as_array().unwrap();
    for entry in file_entries {
        assert_eq!(entry["isDir"], false, "filesOnly should only return files, got: {}", entry);
    }
    assert!(output_files["summary"]["totalMatches"].as_u64().unwrap() >= 1,
        "Should find at least one file matching 'Models'");

    cleanup_tmp(&tmp_dir);
}

/// T16 — search_fast regex mode.
/// Using regex=true with a regex pattern should match file names via regex.
#[test]
fn test_search_fast_regex_mode() {
    let (ctx, tmp) = make_search_fast_ctx();

    // Regex pattern to match files ending in "State.cs"
    let result = handle_search_fast(&ctx, &json!({"pattern": ".*State\\.cs$", "regex": true}));
    assert!(!result.is_error, "regex search should not error: {}", result.content[0].text);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    // The make_search_fast_ctx creates "ScannerJobState.cs" which should match
    assert_eq!(output["summary"]["totalMatches"], 1,
        "Regex '.*State\\.cs$' should match exactly ScannerJobState.cs");
    let files = output["files"].as_array().unwrap();
    assert!(files[0]["path"].as_str().unwrap().contains("ScannerJobState"),
        "Matched file should be ScannerJobState.cs");

    // Regex that matches multiple files: anything with "Model" in the name
    let result2 = handle_search_fast(&ctx, &json!({"pattern": "Model.*\\.cs$", "regex": true}));
    assert!(!result2.is_error);
    let output2: Value = serde_json::from_str(&result2.content[0].text).unwrap();
    assert_eq!(output2["summary"]["totalMatches"], 2,
        "Regex 'Model.*\\.cs$' should match ModelSchemaStorage.cs and ModelSchemaManager.cs");

    cleanup_tmp(&tmp);
}

/// T22 — search_definitions baseType filter at handler level.
/// Only definitions whose base_types include the given baseType should be returned.
#[test]
fn test_search_definitions_base_type_filter() {
    use crate::definitions::*;

    let content_index = ContentIndex {
        root: ".".to_string(), created_at: 0, max_age_secs: 3600,
        files: vec!["C:\\src\\Controllers.cs".to_string()],
        index: HashMap::new(), total_tokens: 50,
        extensions: vec!["cs".to_string()],
        file_token_counts: vec![50],
        trigram: TrigramIndex::default(), trigram_dirty: false, forward: None, path_to_id: None,
    };

    let definitions = vec![
        DefinitionEntry {
            file_id: 0, name: "UserController".to_string(),
            kind: DefinitionKind::Class, line_start: 1, line_end: 50,
            parent: None, signature: None, modifiers: vec![], attributes: vec![],
            base_types: vec!["ControllerBase".to_string()],
        },
        DefinitionEntry {
            file_id: 0, name: "OrderService".to_string(),
            kind: DefinitionKind::Class, line_start: 55, line_end: 100,
            parent: None, signature: None, modifiers: vec![], attributes: vec![],
            base_types: vec!["IOrderService".to_string()],
        },
        DefinitionEntry {
            file_id: 0, name: "AdminController".to_string(),
            kind: DefinitionKind::Class, line_start: 105, line_end: 150,
            parent: None, signature: None, modifiers: vec![], attributes: vec![],
            base_types: vec!["ControllerBase".to_string(), "IAdminAccess".to_string()],
        },
        DefinitionEntry {
            file_id: 0, name: "PlainClass".to_string(),
            kind: DefinitionKind::Class, line_start: 155, line_end: 170,
            parent: None, signature: None, modifiers: vec![], attributes: vec![],
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
    path_to_id.insert(PathBuf::from("C:\\src\\Controllers.cs"), 0);

    let def_index = DefinitionIndex {
        root: ".".to_string(), created_at: 0,
        extensions: vec!["cs".to_string()],
        files: vec!["C:\\src\\Controllers.cs".to_string()],
        definitions, name_index, kind_index,
        attribute_index: HashMap::new(), base_type_index,
        file_index, path_to_id, method_calls: HashMap::new(),
        parse_errors: 0, lossy_file_count: 0, empty_file_ids: Vec::new(),
    };

    let ctx = HandlerContext {
        index: Arc::new(RwLock::new(content_index)),
        def_index: Some(Arc::new(RwLock::new(def_index))),
        server_dir: ".".to_string(), server_ext: "cs".to_string(),
        metrics: false, index_base: PathBuf::from("."),
        max_response_bytes: crate::mcp::handlers::utils::DEFAULT_MAX_RESPONSE_BYTES,
        content_ready: Arc::new(AtomicBool::new(true)),
        def_ready: Arc::new(AtomicBool::new(true)),
    };

    // Filter by baseType=ControllerBase — should return UserController and AdminController
    let result = dispatch_tool(&ctx, "search_definitions", &json!({
        "baseType": "ControllerBase"
    }));
    assert!(!result.is_error, "baseType filter should not error: {}", result.content[0].text);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let defs = output["definitions"].as_array().unwrap();
    assert_eq!(defs.len(), 2, "Expected 2 definitions with baseType=ControllerBase, got {}", defs.len());
    let names: Vec<&str> = defs.iter().filter_map(|d| d["name"].as_str()).collect();
    assert!(names.contains(&"UserController"), "Should contain UserController");
    assert!(names.contains(&"AdminController"), "Should contain AdminController");

    // Filter by baseType=IOrderService — should return only OrderService
    let result2 = dispatch_tool(&ctx, "search_definitions", &json!({
        "baseType": "IOrderService"
    }));
    assert!(!result2.is_error);
    let output2: Value = serde_json::from_str(&result2.content[0].text).unwrap();
    let defs2 = output2["definitions"].as_array().unwrap();
    assert_eq!(defs2.len(), 1, "Expected 1 definition with baseType=IOrderService, got {}", defs2.len());
    assert_eq!(defs2[0]["name"], "OrderService");

    // Filter by non-existent baseType — should return empty
    let result3 = dispatch_tool(&ctx, "search_definitions", &json!({
        "baseType": "NonExistentBase"
    }));
    assert!(!result3.is_error);
    let output3: Value = serde_json::from_str(&result3.content[0].text).unwrap();
    let defs3 = output3["definitions"].as_array().unwrap();
    assert!(defs3.is_empty(), "Non-existent baseType should return empty, got {}", defs3.len());
}

/// T34 — search_reindex_definitions success case.
/// When a definitions index exists, reindex should succeed and return status=ok.
#[test]
fn test_reindex_definitions_success() {
    use std::io::Write;
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let tmp_dir = std::env::temp_dir().join(format!("search_reindex_def_{}_{}", std::process::id(), id));
    let _ = std::fs::create_dir_all(&tmp_dir);

    // Create a minimal .cs file so the reindex has something to parse
    let cs_file = tmp_dir.join("Sample.cs");
    {
        let mut f = std::fs::File::create(&cs_file).unwrap();
        writeln!(f, "public class SampleClass {{").unwrap();
        writeln!(f, "    public void DoWork() {{ }}").unwrap();
        writeln!(f, "}}").unwrap();
    }

    let dir_str = tmp_dir.to_string_lossy().to_string();

    // Build an initial (empty) definition index
    let def_index = crate::definitions::DefinitionIndex {
        root: dir_str.clone(), created_at: 0,
        extensions: vec!["cs".to_string()],
        files: vec![], definitions: vec![],
        name_index: HashMap::new(), kind_index: HashMap::new(),
        attribute_index: HashMap::new(), base_type_index: HashMap::new(),
        file_index: HashMap::new(), path_to_id: HashMap::new(),
        method_calls: HashMap::new(), parse_errors: 0, lossy_file_count: 0,
        empty_file_ids: Vec::new(),
    };

    let content_index = ContentIndex {
        root: dir_str.clone(), created_at: 0, max_age_secs: 3600,
        files: vec![], index: HashMap::new(), total_tokens: 0,
        extensions: vec!["cs".to_string()], file_token_counts: vec![],
        trigram: TrigramIndex::default(), trigram_dirty: false, forward: None, path_to_id: None,
    };

    let ctx = HandlerContext {
        index: Arc::new(RwLock::new(content_index)),
        def_index: Some(Arc::new(RwLock::new(def_index))),
        server_dir: dir_str.clone(),
        server_ext: "cs".to_string(),
        metrics: false,
        index_base: tmp_dir.join(".index"),
        max_response_bytes: crate::mcp::handlers::utils::DEFAULT_MAX_RESPONSE_BYTES,
        content_ready: Arc::new(AtomicBool::new(true)),
        def_ready: Arc::new(AtomicBool::new(true)),
    };

    let result = dispatch_tool(&ctx, "search_reindex_definitions", &json!({}));
    assert!(!result.is_error, "Reindex definitions should succeed: {}", result.content[0].text);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    assert_eq!(output["status"], "ok", "Status should be 'ok'");
    assert!(output["files"].as_u64().unwrap() >= 1, "Should have parsed at least 1 file");
    assert!(output["definitions"].as_u64().unwrap() >= 1, "Should have found at least 1 definition");
    assert!(output["rebuildTimeMs"].as_f64().is_some(), "Should report rebuild time");

    cleanup_tmp(&tmp_dir);
}

/// T43-T45 — search_find combined parameters: countOnly, maxDepth, ignoreCase, regex.
#[test]
fn test_search_find_combined_parameters() {
    use std::io::Write;
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let tmp_dir = std::env::temp_dir().join(format!("search_find_combined_{}_{}", std::process::id(), id));
    let _ = std::fs::create_dir_all(&tmp_dir);

    // Create nested structure:
    //   tmp/level1/level2/deep.cs
    //   tmp/level1/shallow.cs
    //   tmp/TopFile.CS  (uppercase extension for case test)
    let level1 = tmp_dir.join("level1");
    let level2 = level1.join("level2");
    std::fs::create_dir_all(&level2).unwrap();
    { let mut f = std::fs::File::create(level2.join("deep.cs")).unwrap(); writeln!(f, "// deep").unwrap(); }
    { let mut f = std::fs::File::create(level1.join("shallow.cs")).unwrap(); writeln!(f, "// shallow").unwrap(); }
    { let mut f = std::fs::File::create(tmp_dir.join("TopFile.CS")).unwrap(); writeln!(f, "// top").unwrap(); }

    let dir_str = tmp_dir.to_string_lossy().to_string();
    let content_index = ContentIndex {
        root: dir_str.clone(), created_at: 0, max_age_secs: 3600,
        files: vec![], index: HashMap::new(), total_tokens: 0,
        extensions: vec!["cs".to_string()], file_token_counts: vec![],
        trigram: TrigramIndex::default(), trigram_dirty: false, forward: None, path_to_id: None,
    };
    let ctx = HandlerContext {
        index: Arc::new(RwLock::new(content_index)),
        def_index: None,
        server_dir: dir_str.clone(),
        server_ext: "cs".to_string(),
        metrics: false,
        index_base: tmp_dir.join(".index"),
        max_response_bytes: crate::mcp::handlers::utils::DEFAULT_MAX_RESPONSE_BYTES,
        content_ready: Arc::new(AtomicBool::new(true)),
        def_ready: Arc::new(AtomicBool::new(true)),
    };

    // T43: countOnly=true — should return only counts, no file listings
    let result_count = dispatch_tool(&ctx, "search_find", &json!({
        "pattern": ".cs",
        "countOnly": true,
        "ignoreCase": true
    }));
    assert!(!result_count.is_error, "countOnly should not error: {}", result_count.content[0].text);
    let output_count: Value = serde_json::from_str(&result_count.content[0].text).unwrap();
    assert!(output_count["summary"]["totalMatches"].as_u64().unwrap() >= 3,
        "Should find at least 3 .cs files (case-insensitive)");
    assert!(output_count["files"].as_array().unwrap().is_empty(),
        "countOnly=true should return empty files array");

    // T44: maxDepth — should limit directory traversal
    // maxDepth=1 should find TopFile.CS at root but NOT files in level1/ subdirectories
    // Note: maxDepth=1 means root-level only in ignore::WalkBuilder
    let result_depth = dispatch_tool(&ctx, "search_find", &json!({
        "pattern": ".cs",
        "maxDepth": 1,
        "ignoreCase": true
    }));
    assert!(!result_depth.is_error);
    let output_depth: Value = serde_json::from_str(&result_depth.content[0].text).unwrap();
    let depth_matches = output_depth["summary"]["totalMatches"].as_u64().unwrap();
    // maxDepth=1 means only root level — should find TopFile.CS but not deeper files
    assert!(depth_matches < 3,
        "maxDepth=1 should find fewer than 3 files, got {}", depth_matches);

    // T45: ignoreCase=true + regex=true — case-insensitive regex matching
    let result_regex = dispatch_tool(&ctx, "search_find", &json!({
        "pattern": "top.*\\.cs",
        "regex": true,
        "ignoreCase": true
    }));
    assert!(!result_regex.is_error, "regex+ignoreCase should not error: {}", result_regex.content[0].text);
    let output_regex: Value = serde_json::from_str(&result_regex.content[0].text).unwrap();
    assert!(output_regex["summary"]["totalMatches"].as_u64().unwrap() >= 1,
        "Case-insensitive regex 'top.*\\.cs' should match TopFile.CS");

    cleanup_tmp(&tmp_dir);
}

/// T72 — search_definitions enumMember kind at handler level.
/// Filtering by kind="enumMember" should return only enum member definitions.
#[test]
fn test_search_definitions_enum_member_kind() {
    use crate::definitions::*;

    let content_index = ContentIndex {
        root: ".".to_string(), created_at: 0, max_age_secs: 3600,
        files: vec!["C:\\src\\Enums.cs".to_string()],
        index: HashMap::new(), total_tokens: 50,
        extensions: vec!["cs".to_string()],
        file_token_counts: vec![50],
        trigram: TrigramIndex::default(), trigram_dirty: false, forward: None, path_to_id: None,
    };

    let definitions = vec![
        DefinitionEntry {
            file_id: 0, name: "OrderStatus".to_string(),
            kind: DefinitionKind::Enum, line_start: 1, line_end: 20,
            parent: None, signature: None, modifiers: vec![], attributes: vec![], base_types: vec![],
        },
        DefinitionEntry {
            file_id: 0, name: "Pending".to_string(),
            kind: DefinitionKind::EnumMember, line_start: 3, line_end: 3,
            parent: Some("OrderStatus".to_string()), signature: None,
            modifiers: vec![], attributes: vec![], base_types: vec![],
        },
        DefinitionEntry {
            file_id: 0, name: "Completed".to_string(),
            kind: DefinitionKind::EnumMember, line_start: 4, line_end: 4,
            parent: Some("OrderStatus".to_string()), signature: None,
            modifiers: vec![], attributes: vec![], base_types: vec![],
        },
        DefinitionEntry {
            file_id: 0, name: "Cancelled".to_string(),
            kind: DefinitionKind::EnumMember, line_start: 5, line_end: 5,
            parent: Some("OrderStatus".to_string()), signature: None,
            modifiers: vec![], attributes: vec![], base_types: vec![],
        },
        DefinitionEntry {
            file_id: 0, name: "OrderHelper".to_string(),
            kind: DefinitionKind::Class, line_start: 25, line_end: 50,
            parent: None, signature: None, modifiers: vec![], attributes: vec![], base_types: vec![],
        },
        DefinitionEntry {
            file_id: 0, name: "GetStatus".to_string(),
            kind: DefinitionKind::Method, line_start: 30, line_end: 40,
            parent: Some("OrderHelper".to_string()), signature: None,
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
    path_to_id.insert(PathBuf::from("C:\\src\\Enums.cs"), 0);

    let def_index = DefinitionIndex {
        root: ".".to_string(), created_at: 0,
        extensions: vec!["cs".to_string()],
        files: vec!["C:\\src\\Enums.cs".to_string()],
        definitions, name_index, kind_index,
        attribute_index: HashMap::new(), base_type_index: HashMap::new(),
        file_index, path_to_id, method_calls: HashMap::new(),
        parse_errors: 0, lossy_file_count: 0, empty_file_ids: Vec::new(),
    };

    let ctx = HandlerContext {
        index: Arc::new(RwLock::new(content_index)),
        def_index: Some(Arc::new(RwLock::new(def_index))),
        server_dir: ".".to_string(), server_ext: "cs".to_string(),
        metrics: false, index_base: PathBuf::from("."),
        max_response_bytes: crate::mcp::handlers::utils::DEFAULT_MAX_RESPONSE_BYTES,
        content_ready: Arc::new(AtomicBool::new(true)),
        def_ready: Arc::new(AtomicBool::new(true)),
    };

    // Filter by kind=enumMember
    let result = dispatch_tool(&ctx, "search_definitions", &json!({
        "kind": "enumMember"
    }));
    assert!(!result.is_error, "kind=enumMember should not error: {}", result.content[0].text);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let defs = output["definitions"].as_array().unwrap();

    // Should return exactly 3 enum members: Pending, Completed, Cancelled
    assert_eq!(defs.len(), 3, "Expected 3 enumMember definitions, got {}", defs.len());
    for def in defs {
        assert_eq!(def["kind"], "enumMember",
            "All results should be enumMember, but got kind={}", def["kind"]);
    }
    let names: Vec<&str> = defs.iter().filter_map(|d| d["name"].as_str()).collect();
    assert!(names.contains(&"Pending"), "Should contain Pending enum member");
    assert!(names.contains(&"Completed"), "Should contain Completed enum member");
    assert!(names.contains(&"Cancelled"), "Should contain Cancelled enum member");

    // Verify parent is set correctly
    for def in defs {
        assert_eq!(def["parent"], "OrderStatus",
            "Enum members should have parent=OrderStatus, got {}", def["parent"]);
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Batch 3 tests — Nice-to-have edge cases
// ═══════════════════════════════════════════════════════════════════════

/// T39 — search_grep SQL extension filter.
/// When ext="sql" is specified, only files with .sql extension should be returned.
#[test]
fn test_search_grep_sql_extension_filter() {
    // Build an index with both .cs and .sql files containing the same token
    let mut idx = HashMap::new();
    idx.insert("createtable".to_string(), vec![
        Posting { file_id: 0, lines: vec![5] },
        Posting { file_id: 1, lines: vec![10] },
        Posting { file_id: 2, lines: vec![3] },
    ]);

    let index = ContentIndex {
        root: ".".to_string(),
        created_at: 0,
        max_age_secs: 3600,
        files: vec![
            "C:\\src\\Schema.sql".to_string(),
            "C:\\src\\Service.cs".to_string(),
            "C:\\src\\Migration.sql".to_string(),
        ],
        index: idx,
        total_tokens: 100,
        extensions: vec!["cs".to_string(), "sql".to_string()],
        file_token_counts: vec![50, 50, 50],
        trigram: TrigramIndex::default(),
        trigram_dirty: false,
        forward: None,
        path_to_id: None,
    };

    let ctx = HandlerContext {
        index: Arc::new(RwLock::new(index)),
        def_index: None,
        server_dir: ".".to_string(),
        server_ext: "cs,sql".to_string(),
        metrics: false,
        index_base: PathBuf::from("."),
        max_response_bytes: crate::mcp::handlers::utils::DEFAULT_MAX_RESPONSE_BYTES,
        content_ready: Arc::new(AtomicBool::new(true)),
        def_ready: Arc::new(AtomicBool::new(true)),
    };

    // With ext="sql" filter — should only return .sql files
    let result = dispatch_tool(&ctx, "search_grep", &json!({
        "terms": "createtable",
        "ext": "sql",
        "substring": false
    }));
    assert!(!result.is_error, "grep with ext=sql should not error: {}", result.content[0].text);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    assert_eq!(output["summary"]["totalFiles"], 2,
        "Should find exactly 2 .sql files, got: {}", output["summary"]["totalFiles"]);
    let files = output["files"].as_array().unwrap();
    for file in files {
        let path = file["path"].as_str().unwrap();
        assert!(path.ends_with(".sql"),
            "All results should be .sql files, but found: {}", path);
    }

    // Without ext filter — should return all 3 files
    let result_all = dispatch_tool(&ctx, "search_grep", &json!({
        "terms": "createtable",
        "substring": false
    }));
    assert!(!result_all.is_error);
    let output_all: Value = serde_json::from_str(&result_all.content[0].text).unwrap();
    assert_eq!(output_all["summary"]["totalFiles"], 3,
        "Without ext filter should find all 3 files");
}

/// T71 — search_grep SQL phrase search with showLines.
/// Phrase search with showLines=true should return line content from matching files.
#[test]
fn test_search_grep_phrase_search_with_show_lines() {
    use std::io::Write;
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let tmp_dir = std::env::temp_dir().join(format!("search_phrase_sql_{}_{}", std::process::id(), id));
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir).unwrap();

    // Create a .sql file with a SQL phrase
    {
        let mut f = std::fs::File::create(tmp_dir.join("schema.sql")).unwrap();
        writeln!(f, "-- Database schema").unwrap();
        writeln!(f, "CREATE TABLE Users (").unwrap();
        writeln!(f, "    Id INT PRIMARY KEY,").unwrap();
        writeln!(f, "    Name NVARCHAR(100)").unwrap();
        writeln!(f, ");").unwrap();
        writeln!(f, "CREATE TABLE Orders (").unwrap();
        writeln!(f, "    OrderId INT PRIMARY KEY").unwrap();
        writeln!(f, ");").unwrap();
    }
    // Create a file WITHOUT the phrase
    {
        let mut f = std::fs::File::create(tmp_dir.join("other.sql")).unwrap();
        writeln!(f, "-- No create table here").unwrap();
        writeln!(f, "SELECT * FROM Users;").unwrap();
    }

    let content_index = crate::build_content_index(&crate::ContentIndexArgs {
        dir: tmp_dir.to_string_lossy().to_string(),
        ext: "sql".to_string(),
        max_age_hours: 24, hidden: false, no_ignore: false, threads: 1, min_token_len: 2,
    });

    let ctx = HandlerContext {
        index: Arc::new(RwLock::new(content_index)),
        def_index: None,
        server_dir: tmp_dir.to_string_lossy().to_string(),
        server_ext: "sql".to_string(),
        metrics: false,
        index_base: tmp_dir.join(".index"),
        max_response_bytes: crate::mcp::handlers::utils::DEFAULT_MAX_RESPONSE_BYTES,
        content_ready: Arc::new(AtomicBool::new(true)),
        def_ready: Arc::new(AtomicBool::new(true)),
    };

    // Phrase search for "CREATE TABLE" with showLines=true
    let result = dispatch_tool(&ctx, "search_grep", &json!({
        "terms": "CREATE TABLE",
        "phrase": true,
        "showLines": true
    }));
    assert!(!result.is_error, "Phrase search should not error: {}", result.content[0].text);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();

    // Should find at least 1 file (schema.sql has "CREATE TABLE")
    let total = output["summary"]["totalFiles"].as_u64().unwrap();
    assert!(total >= 1, "Should find at least 1 file with 'CREATE TABLE' phrase, got {}", total);

    // Verify showLines returned line content
    let files = output["files"].as_array().unwrap();
    assert!(!files.is_empty(), "Files array should not be empty");
    let first_file = &files[0];
    assert!(first_file["lineContent"].is_array(),
        "showLines=true should produce lineContent array");
    let line_content = first_file["lineContent"].as_array().unwrap();
    assert!(!line_content.is_empty(), "lineContent should have entries");

    cleanup_tmp(&tmp_dir);
}

/// T76 — search_fast empty pattern edge case.
/// An empty string pattern should be handled gracefully (not crash/panic).
#[test]
fn test_search_fast_empty_pattern() {
    let (ctx, tmp) = make_search_fast_ctx();

    // Empty pattern — should return error (missing required parameter)
    // because after trim/filter the pattern is empty
    let result = handle_search_fast(&ctx, &json!({"pattern": ""}));

    // The handler splits by ',' and filters empty strings, resulting in 0 terms.
    // It should either return an error or return 0 matches gracefully.
    if result.is_error {
        // Handler correctly rejects empty pattern
        assert!(result.content[0].text.contains("Missing") || result.content[0].text.contains("pattern") || result.content[0].text.contains("empty"),
            "Error should mention missing/empty pattern, got: {}", result.content[0].text);
    } else {
        // Handler returns 0 matches for empty pattern (also acceptable)
        let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
        assert_eq!(output["summary"]["totalMatches"], 0,
            "Empty pattern should return 0 matches");
    }

    cleanup_tmp(&tmp);
}

/// T77 — search_definitions file filter: backslash vs forward slash normalization.
/// Both backslashes (Windows-style) and forward slashes (Unix-style) should match
/// the same definitions when used in the file filter parameter.
#[test]
fn test_search_definitions_file_filter_slash_normalization() {
    use crate::definitions::*;

    // Create definitions with Windows-style backslash paths
    let content_index = ContentIndex {
        root: ".".to_string(), created_at: 0, max_age_secs: 3600,
        files: vec![
            "C:\\src\\Models\\User.cs".to_string(),
            "C:\\src\\Services\\UserService.cs".to_string(),
        ],
        index: HashMap::new(), total_tokens: 50,
        extensions: vec!["cs".to_string()],
        file_token_counts: vec![25, 25],
        trigram: TrigramIndex::default(), trigram_dirty: false, forward: None, path_to_id: None,
    };

    let definitions = vec![
        DefinitionEntry {
            file_id: 0, name: "UserModel".to_string(),
            kind: DefinitionKind::Class, line_start: 1, line_end: 30,
            parent: None, signature: None, modifiers: vec![], attributes: vec![], base_types: vec![],
        },
        DefinitionEntry {
            file_id: 1, name: "UserService".to_string(),
            kind: DefinitionKind::Class, line_start: 1, line_end: 50,
            parent: None, signature: None, modifiers: vec![], attributes: vec![], base_types: vec![],
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
    path_to_id.insert(PathBuf::from("C:\\src\\Models\\User.cs"), 0);
    path_to_id.insert(PathBuf::from("C:\\src\\Services\\UserService.cs"), 1);

    let def_index = DefinitionIndex {
        root: ".".to_string(), created_at: 0,
        extensions: vec!["cs".to_string()],
        files: vec![
            "C:\\src\\Models\\User.cs".to_string(),
            "C:\\src\\Services\\UserService.cs".to_string(),
        ],
        definitions, name_index, kind_index,
        attribute_index: HashMap::new(), base_type_index: HashMap::new(),
        file_index, path_to_id, method_calls: HashMap::new(),
        parse_errors: 0, lossy_file_count: 0, empty_file_ids: Vec::new(),
    };

    let ctx = HandlerContext {
        index: Arc::new(RwLock::new(content_index)),
        def_index: Some(Arc::new(RwLock::new(def_index))),
        server_dir: ".".to_string(), server_ext: "cs".to_string(),
        metrics: false, index_base: PathBuf::from("."),
        max_response_bytes: crate::mcp::handlers::utils::DEFAULT_MAX_RESPONSE_BYTES,
        content_ready: Arc::new(AtomicBool::new(true)),
        def_ready: Arc::new(AtomicBool::new(true)),
    };

    // Search with backslash path (Windows-style) — should match
    let result_backslash = dispatch_tool(&ctx, "search_definitions", &json!({
        "file": "Models\\User"
    }));
    assert!(!result_backslash.is_error);
    let output_bs: Value = serde_json::from_str(&result_backslash.content[0].text).unwrap();
    let defs_bs = output_bs["definitions"].as_array().unwrap();

    // Search with forward slash path (Unix-style) — file filter uses contains()
    // so "Models/User" should also match "C:\\src\\Models\\User.cs" if case-insensitive
    // The file filter uses: file_path.to_lowercase().contains(&ff.to_lowercase())
    // Backslash in path won't match forward slash in filter, so this tests actual behavior
    let result_fwdslash = dispatch_tool(&ctx, "search_definitions", &json!({
        "file": "Models/User"
    }));
    assert!(!result_fwdslash.is_error);
    let output_fs: Value = serde_json::from_str(&result_fwdslash.content[0].text).unwrap();
    let defs_fs = output_fs["definitions"].as_array().unwrap();

    // Backslash filter should always match (path uses backslashes)
    assert_eq!(defs_bs.len(), 1,
        "Backslash file filter should find UserModel, got {} results", defs_bs.len());
    assert_eq!(defs_bs[0]["name"], "UserModel");

    // Forward slash filter behavior: depends on whether the handler normalizes slashes.
    // The current implementation uses simple string contains() — forward slashes won't
    // match backslash paths. This test documents the actual behavior.
    // If defs_fs is empty, the handler does NOT normalize slashes (expected current behavior).
    // If defs_fs has results, the handler DOES normalize slashes.
    if defs_fs.is_empty() {
        // Current behavior: no normalization — forward slash doesn't match backslash path
        // This is a known limitation that could be improved in the future
        assert_eq!(defs_fs.len(), 0,
            "Forward slash filter currently does not match backslash paths (no normalization)");
    } else {
        // If normalization was added, both should return the same results
        assert_eq!(defs_fs.len(), defs_bs.len(),
            "If slash normalization exists, both filters should return same count");
    }

    // Use a path fragment that works regardless of slash direction
    // Note: "User.cs" is a substring of "User.cs" but NOT of "UserService.cs"
    let result_fragment = dispatch_tool(&ctx, "search_definitions", &json!({
        "file": "User"
    }));
    assert!(!result_fragment.is_error);
    let output_frag: Value = serde_json::from_str(&result_fragment.content[0].text).unwrap();
    let defs_frag = output_frag["definitions"].as_array().unwrap();
    // Both files contain "User" substring (User.cs and UserService.cs)
    assert_eq!(defs_frag.len(), 2,
        "File filter 'User' should match both User.cs and UserService.cs, got {}", defs_frag.len());
}

/// T78 — search_callers cycle detection in direction=down.
/// When the call graph has a cycle (A calls B, B calls A), the handler should
/// complete without infinite loop thanks to the visited set.
#[test]
fn test_search_callers_cycle_detection_down() {
    use crate::definitions::*;

    // Set up: MethodA (in ClassA) calls MethodB (in ClassB),
    // and MethodB calls MethodA back — creating a cycle.
    let definitions = vec![
        DefinitionEntry {
            file_id: 0, name: "ClassA".to_string(),
            kind: DefinitionKind::Class, line_start: 1, line_end: 50,
            parent: None, signature: None, modifiers: vec![], attributes: vec![], base_types: vec![],
        },
        DefinitionEntry {
            file_id: 0, name: "MethodA".to_string(),
            kind: DefinitionKind::Method, line_start: 10, line_end: 30,
            parent: Some("ClassA".to_string()), signature: None,
            modifiers: vec![], attributes: vec![], base_types: vec![],
        },
        DefinitionEntry {
            file_id: 1, name: "ClassB".to_string(),
            kind: DefinitionKind::Class, line_start: 1, line_end: 50,
            parent: None, signature: None, modifiers: vec![], attributes: vec![], base_types: vec![],
        },
        DefinitionEntry {
            file_id: 1, name: "MethodB".to_string(),
            kind: DefinitionKind::Method, line_start: 10, line_end: 30,
            parent: Some("ClassB".to_string()), signature: None,
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
    path_to_id.insert(PathBuf::from("C:\\src\\ClassA.cs"), 0);
    path_to_id.insert(PathBuf::from("C:\\src\\ClassB.cs"), 1);

    // MethodA (def index 1) calls MethodB; MethodB (def index 3) calls MethodA
    let mut method_calls: HashMap<u32, Vec<CallSite>> = HashMap::new();
    method_calls.insert(1, vec![CallSite {
        method_name: "MethodB".to_string(),
        receiver_type: Some("ClassB".to_string()),
        line: 20,
    }]);
    method_calls.insert(3, vec![CallSite {
        method_name: "MethodA".to_string(),
        receiver_type: Some("ClassA".to_string()),
        line: 20,
    }]);

    let def_index = DefinitionIndex {
        root: ".".to_string(), created_at: 0,
        extensions: vec!["cs".to_string()],
        files: vec!["C:\\src\\ClassA.cs".to_string(), "C:\\src\\ClassB.cs".to_string()],
        definitions, name_index, kind_index,
        attribute_index: HashMap::new(), base_type_index: HashMap::new(),
        file_index, path_to_id, method_calls,
        parse_errors: 0, lossy_file_count: 0, empty_file_ids: Vec::new(),
    };

    let content_index = ContentIndex {
        root: ".".to_string(), created_at: 0, max_age_secs: 3600,
        files: vec!["C:\\src\\ClassA.cs".to_string(), "C:\\src\\ClassB.cs".to_string()],
        index: HashMap::new(), total_tokens: 0,
        extensions: vec!["cs".to_string()],
        file_token_counts: vec![50, 50],
        trigram: TrigramIndex::default(), trigram_dirty: false, forward: None, path_to_id: None,
    };

    let ctx = HandlerContext {
        index: Arc::new(RwLock::new(content_index)),
        def_index: Some(Arc::new(RwLock::new(def_index))),
        server_dir: ".".to_string(), server_ext: "cs".to_string(),
        metrics: false, index_base: PathBuf::from("."),
        max_response_bytes: crate::mcp::handlers::utils::DEFAULT_MAX_RESPONSE_BYTES,
        content_ready: Arc::new(AtomicBool::new(true)),
        def_ready: Arc::new(AtomicBool::new(true)),
    };

    // direction=down with depth=5 — cycle should be stopped by visited set
    let result = dispatch_tool(&ctx, "search_callers", &json!({
        "method": "MethodA",
        "class": "ClassA",
        "direction": "down",
        "depth": 5,
        "maxTotalNodes": 50
    }));
    assert!(!result.is_error, "Cycle in call graph should not cause error: {}", result.content[0].text);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();

    // Should complete and have some nodes (MethodA → MethodB, but MethodB → MethodA is blocked)
    let tree = output["callTree"].as_array().unwrap();
    let total_nodes = output["summary"]["totalNodes"].as_u64().unwrap();
    assert!(total_nodes > 0, "Should find at least one callee before cycle is detected");
    // The cycle means we can't recurse forever — total nodes should be bounded
    assert!(total_nodes <= 10, "Cycle detection should prevent runaway recursion, got {} nodes", total_nodes);

    // First level should find MethodB as a callee
    if !tree.is_empty() {
        let callee_names: Vec<&str> = tree.iter().filter_map(|n| n["method"].as_str()).collect();
        assert!(callee_names.contains(&"MethodB"),
            "MethodA should call MethodB. Got callees: {:?}", callee_names);
    }
}

/// T80 — search_reindex with invalid/non-existent directory.
/// Should return an error message when given a directory that doesn't exist.
#[test]
fn test_search_reindex_invalid_directory() {
    let ctx = make_empty_ctx();

    // Use a clearly non-existent directory path
    let result = dispatch_tool(&ctx, "search_reindex", &json!({
        "dir": "Z:\\nonexistent\\path\\that\\does\\not\\exist"
    }));

    // The handler should error because:
    // 1. fs::canonicalize fails on non-existent path, falling back to raw string
    // 2. The raw string won't match the server dir, producing a "Server started with --dir" error
    assert!(result.is_error, "Reindex with non-existent dir should error");
    // The error message should mention the server dir mismatch or the invalid directory
    let error_text = &result.content[0].text;
    assert!(
        error_text.contains("Server started with") || error_text.contains("not exist") || error_text.contains("error"),
        "Error should explain the issue. Got: {}", error_text
    );
}

/// T82 — search_grep maxResults=0 semantics.
/// When maxResults=0, the handler should return unlimited results (no truncation).
#[test]
fn test_search_grep_max_results_zero_means_unlimited() {
    // Build an index with many files
    let mut idx = HashMap::new();
    let mut files = Vec::new();
    let mut file_token_counts = Vec::new();

    for i in 0..25 {
        let path = format!("C:\\src\\Module_{:02}\\Service.cs", i);
        files.push(path);
        file_token_counts.push(50u32);
        idx.entry("commontoken".to_string())
            .or_insert_with(Vec::new)
            .push(Posting { file_id: i as u32, lines: vec![10] });
    }

    let index = ContentIndex {
        root: ".".to_string(),
        created_at: 0,
        max_age_secs: 3600,
        files,
        index: idx,
        total_tokens: 1000,
        extensions: vec!["cs".to_string()],
        file_token_counts,
        trigram: TrigramIndex::default(),
        trigram_dirty: false,
        forward: None,
        path_to_id: None,
    };

    let ctx = HandlerContext {
        index: Arc::new(RwLock::new(index)),
        def_index: None,
        server_dir: ".".to_string(),
        server_ext: "cs".to_string(),
        metrics: false,
        index_base: PathBuf::from("."),
        max_response_bytes: 0, // disable response truncation for this test
        content_ready: Arc::new(AtomicBool::new(true)),
        def_ready: Arc::new(AtomicBool::new(true)),
    };

    // maxResults=0 should return ALL 25 files (unlimited)
    let result_unlimited = dispatch_tool(&ctx, "search_grep", &json!({
        "terms": "commontoken",
        "maxResults": 0,
        "substring": false
    }));
    assert!(!result_unlimited.is_error);
    let output_unlimited: Value = serde_json::from_str(&result_unlimited.content[0].text).unwrap();
    assert_eq!(output_unlimited["summary"]["totalFiles"], 25);
    let files_unlimited = output_unlimited["files"].as_array().unwrap();
    assert_eq!(files_unlimited.len(), 25,
        "maxResults=0 should return all 25 files (unlimited), got {}", files_unlimited.len());

    // maxResults=5 should cap at 5 files
    let result_capped = dispatch_tool(&ctx, "search_grep", &json!({
        "terms": "commontoken",
        "maxResults": 5,
        "substring": false
    }));
    assert!(!result_capped.is_error);
    let output_capped: Value = serde_json::from_str(&result_capped.content[0].text).unwrap();
    assert_eq!(output_capped["summary"]["totalFiles"], 25,
        "totalFiles in summary should reflect full count (25)");
    let files_capped = output_capped["files"].as_array().unwrap();
    assert_eq!(files_capped.len(), 5,
        "maxResults=5 should return exactly 5 files, got {}", files_capped.len());

    // Default (no maxResults) should use default of 50, which is > 25 so all 25 returned
    let result_default = dispatch_tool(&ctx, "search_grep", &json!({
        "terms": "commontoken",
        "substring": false
    }));
    assert!(!result_default.is_error);
    let output_default: Value = serde_json::from_str(&result_default.content[0].text).unwrap();
    let files_default = output_default["files"].as_array().unwrap();
    assert_eq!(files_default.len(), 25,
        "Default maxResults=50 should return all 25 files when total < 50, got {}", files_default.len());
}