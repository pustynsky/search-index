//! Tests for MCP handlers — extracted from the original monolithic handlers.rs.
//! Uses `#[path = "handlers_tests.rs"]` in mod.rs to include as the tests module.
//!
//! C#-specific tests (definitions, callers, includeBody, containsLine, reindex)
//! are in handlers_tests_csharp.rs.

use super::*;
use super::grep::handle_search_grep;
use super::fast::handle_search_fast;
use super::utils::validate_search_dir;
use super::handlers_test_utils::{cleanup_tmp, make_ctx_with_defs};
use crate::index::build_trigram_index;
use crate::Posting;
use crate::TrigramIndex;
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

// --- search_callers error tests (general) ---

#[test]
fn test_search_callers_no_def_index() {
    let ctx = make_empty_ctx();
    let result = dispatch_tool(&ctx, "search_callers", &json!({"method": "Foo"}));
    assert!(result.is_error);
    assert!(result.content[0].text.contains("Definition index not available"));
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

// --- containsLine error test (general) ---

#[test]
fn test_contains_line_requires_file() {
    let ctx = make_ctx_with_defs();
    let result = dispatch_tool(&ctx, "search_definitions", &json!({
        "containsLine": 391
    }));
    assert!(result.is_error);
    assert!(result.content[0].text.contains("containsLine requires 'file' parameter"));
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
    assert_eq!(output["summary"]["totalFiles"], 0,
        "AND mode should require ALL terms to match, not count per-token. Got: {}", output);
}

#[test] fn test_substring_and_mode_correct_when_both_terms_match() {
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
    let ctx = make_substring_ctx(
        vec![
            ("catalogquerymanager", 0, vec![39]),
            ("icatalogquerymanager", 1, vec![5]),
            ("m_catalogquerymanager", 2, vec![12]),
        ],
        vec!["C:\\test\\CatalogQueryManager.cs", "C:\\test\\ICatalogQueryManager.cs", "C:\\test\\Controller.cs"],
    );
    let result = dispatch_tool(&ctx, "search_grep", &json!({"terms": "CatalogQueryManager"}));
    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    assert_eq!(output["summary"]["totalFiles"], 3,
        "Default substring=true should find compound identifiers. Got: {}", output);
    let mode = output["summary"]["searchMode"].as_str().unwrap();
    assert!(mode.starts_with("substring"), "Expected substring search mode, got: {}", mode);
}

#[test] fn test_substring_false_misses_compound_identifiers() {
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
    assert_eq!(output["summary"]["totalFiles"], 1,
        "substring=false should only find exact token match. Got: {}", output);
}

#[test] fn test_regex_auto_disables_substring() {
    let ctx = make_substring_ctx(
        vec![("httpclient", 0, vec![5])],
        vec!["C:\\test\\Program.cs"],
    );
    let result = dispatch_tool(&ctx, "search_grep", &json!({"terms": "http.*", "regex": true}));
    assert!(!result.is_error, "regex=true should auto-disable substring, not error");
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    assert_eq!(output["summary"]["totalFiles"], 1);
}

#[test] fn test_phrase_auto_disables_substring() {
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

    { let mut f = std::fs::File::create(tmp_dir.join("manifest.xml")).unwrap();
      writeln!(f, "<Root>").unwrap();
      writeln!(f, "  <Property Name=\"A\">value</Property> </Property>").unwrap();
      writeln!(f, "  <Other>text</Other>").unwrap();
      writeln!(f, "</Root>").unwrap(); }

    { let mut f = std::fs::File::create(tmp_dir.join("Service.xml")).unwrap();
      writeln!(f, "<Root>").unwrap();
      writeln!(f, "  <Property Name=\"X\">").unwrap();
      writeln!(f, "    <Property Name=\"Y\">inner</Property>").unwrap();
      writeln!(f, "  </Property>").unwrap();
      writeln!(f, "</Root>").unwrap(); }

    { let mut f = std::fs::File::create(tmp_dir.join("Logger.xml")).unwrap();
      writeln!(f, "<Config>").unwrap();
      writeln!(f, "  <Type>ILogger<string></Type>").unwrap();
      writeln!(f, "</Config>").unwrap(); }

    { let mut f = std::fs::File::create(tmp_dir.join("Other.xml")).unwrap();
      writeln!(f, "<Config>").unwrap();
      writeln!(f, "  <Type>ILogger string adapter</Type>").unwrap();
      writeln!(f, "</Config>").unwrap(); }

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

// --- Response truncation integration tests ---

#[test]
fn test_response_truncation_triggers_on_large_result() {
    let mut idx = HashMap::new();
    let mut files = Vec::new();
    let mut file_token_counts = Vec::new();

    for i in 0..500 {
        let path = format!(
            "C:\\Projects\\Enterprise\\Solution\\src\\Features\\Module_{:03}\\SubModule\\Implementations\\Component_{:03}Service.cs",
            i / 10, i
        );
        files.push(path);
        file_token_counts.push(1000u32);

        let lines: Vec<u32> = (1..=100).collect();
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
        metrics: true,
        index_base: PathBuf::from("."), max_response_bytes: crate::mcp::handlers::utils::DEFAULT_MAX_RESPONSE_BYTES, content_ready: Arc::new(AtomicBool::new(true)), def_ready: Arc::new(AtomicBool::new(true)),
    };

    let result = dispatch_tool(&ctx, "search_grep", &json!({
        "terms": "common",
        "maxResults": 0,
        "substring": false
    }));

    assert!(!result.is_error);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();

    assert_eq!(output["summary"]["totalFiles"], 500);

    assert_eq!(output["summary"]["responseTruncated"], true,
        "Expected responseTruncated=true for 500-file response");
    assert!(output["summary"]["truncationReason"].as_str().is_some(),
        "Expected truncationReason in summary");
    assert!(output["summary"]["hint"].as_str().is_some(),
        "Expected hint in summary");

    let files_arr = output["files"].as_array().unwrap();
    assert!(files_arr.len() < 500,
        "Expected files array to be truncated from 500, got {}", files_arr.len());

    let response_bytes = output["summary"]["responseBytes"].as_u64().unwrap();
    assert!(response_bytes < 20_000,
        "Expected responseBytes < 20000, got {}", response_bytes);
}

#[test]
fn test_response_truncation_does_not_trigger_on_small_result() {
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
    let ctx = HandlerContext {
        content_ready: Arc::new(AtomicBool::new(false)),
        ..make_empty_ctx()
    };
    let result = dispatch_tool(&ctx, "search_find", &json!({"pattern": "nonexistent_xyz"}));
    assert!(!result.is_error, "search_find should work during index build");
}

// ─── Response truncation via small budget ──────────────────────────

#[test]
fn test_search_grep_response_truncation_via_small_budget() {
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

    let ctx = HandlerContext {
        index: Arc::new(RwLock::new(index)),
        def_index: None,
        server_dir: ".".to_string(),
        server_ext: "cs".to_string(),
        metrics: true,
        index_base: PathBuf::from("."),
        max_response_bytes: 2_000,
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

    assert_eq!(output["summary"]["totalFiles"], 100);

    assert_eq!(output["summary"]["responseTruncated"], true,
        "Expected responseTruncated=true for 100-file response with 2KB budget");
    assert!(output["summary"]["truncationReason"].as_str().is_some(),
        "Expected truncationReason in summary");

    let files_arr = output["files"].as_array().unwrap();
    assert!(files_arr.len() < 100,
        "Expected files array to be truncated from 100, got {}", files_arr.len());
}

// ─── General search_definitions tests ───────────────────────────────

/// search_definitions non-existent name returns empty.
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

/// search_definitions invalid regex error.
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

// ═══════════════════════════════════════════════════════════════════════
// Batch 2 tests — Strengthen Partial Coverage
// ═══════════════════════════════════════════════════════════════════════

/// T15 — search_fast dirsOnly and filesOnly filters.
#[test]
fn test_search_fast_dirs_only_and_files_only() {
    use std::io::Write;
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let tmp_dir = std::env::temp_dir().join(format!("search_fast_dironly_{}_{}", std::process::id(), id));
    let _ = std::fs::create_dir_all(&tmp_dir);

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

    let result_dirs = handle_search_fast(&ctx, &json!({"pattern": "Models", "dirsOnly": true}));
    assert!(!result_dirs.is_error, "dirsOnly should not error: {}", result_dirs.content[0].text);
    let output_dirs: Value = serde_json::from_str(&result_dirs.content[0].text).unwrap();
    let dir_files = output_dirs["files"].as_array().unwrap();
    for entry in dir_files {
        assert_eq!(entry["isDir"], true, "dirsOnly should only return directories, got: {}", entry);
    }
    assert!(output_dirs["summary"]["totalMatches"].as_u64().unwrap() >= 1,
        "Should find at least one directory matching 'Models'");

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
#[test]
fn test_search_fast_regex_mode() {
    let (ctx, tmp) = make_search_fast_ctx();

    let result = handle_search_fast(&ctx, &json!({"pattern": ".*State\\.cs$", "regex": true}));
    assert!(!result.is_error, "regex search should not error: {}", result.content[0].text);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    assert_eq!(output["summary"]["totalMatches"], 1,
        "Regex '.*State\\.cs$' should match exactly ScannerJobState.cs");
    let files = output["files"].as_array().unwrap();
    assert!(files[0]["path"].as_str().unwrap().contains("ScannerJobState"),
        "Matched file should be ScannerJobState.cs");

    let result2 = handle_search_fast(&ctx, &json!({"pattern": "Model.*\\.cs$", "regex": true}));
    assert!(!result2.is_error);
    let output2: Value = serde_json::from_str(&result2.content[0].text).unwrap();
    assert_eq!(output2["summary"]["totalMatches"], 2,
        "Regex 'Model.*\\.cs$' should match ModelSchemaStorage.cs and ModelSchemaManager.cs");

    cleanup_tmp(&tmp);
}

// ═══════════════════════════════════════════════════════════════════════
// Batch 3 tests — Nice-to-have edge cases
// ═══════════════════════════════════════════════════════════════════════

/// T39 — search_grep SQL extension filter.
#[test]
fn test_search_grep_sql_extension_filter() {
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
#[test]
fn test_search_grep_phrase_search_with_show_lines() {
    use std::io::Write;
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let tmp_dir = std::env::temp_dir().join(format!("search_phrase_sql_{}_{}", std::process::id(), id));
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir).unwrap();

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

    let result = dispatch_tool(&ctx, "search_grep", &json!({
        "terms": "CREATE TABLE",
        "phrase": true,
        "showLines": true
    }));
    assert!(!result.is_error, "Phrase search should not error: {}", result.content[0].text);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();

    let total = output["summary"]["totalFiles"].as_u64().unwrap();
    assert!(total >= 1, "Should find at least 1 file with 'CREATE TABLE' phrase, got {}", total);

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
#[test]
fn test_search_fast_empty_pattern() {
    let (ctx, tmp) = make_search_fast_ctx();

    let result = handle_search_fast(&ctx, &json!({"pattern": ""}));

    if result.is_error {
        assert!(result.content[0].text.contains("Missing") || result.content[0].text.contains("pattern") || result.content[0].text.contains("empty"),
            "Error should mention missing/empty pattern, got: {}", result.content[0].text);
    } else {
        let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
        assert_eq!(output["summary"]["totalMatches"], 0,
            "Empty pattern should return 0 matches");
    }

    cleanup_tmp(&tmp);
}

/// T77 — search_definitions file filter: backslash vs forward slash normalization.
#[test]
fn test_search_definitions_file_filter_slash_normalization() {
    use crate::definitions::*;

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
        kind_index.entry(def.kind).or_default().push(idx);
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

    let result_backslash = dispatch_tool(&ctx, "search_definitions", &json!({
        "file": "Models\\User"
    }));
    assert!(!result_backslash.is_error);
    let output_bs: Value = serde_json::from_str(&result_backslash.content[0].text).unwrap();
    let defs_bs = output_bs["definitions"].as_array().unwrap();

    let result_fwdslash = dispatch_tool(&ctx, "search_definitions", &json!({
        "file": "Models/User"
    }));
    assert!(!result_fwdslash.is_error);
    let output_fs: Value = serde_json::from_str(&result_fwdslash.content[0].text).unwrap();
    let defs_fs = output_fs["definitions"].as_array().unwrap();

    assert_eq!(defs_bs.len(), 1,
        "Backslash file filter should find UserModel, got {} results", defs_bs.len());
    assert_eq!(defs_bs[0]["name"], "UserModel");

    if defs_fs.is_empty() {
        assert_eq!(defs_fs.len(), 0,
            "Forward slash filter currently does not match backslash paths (no normalization)");
    } else {
        assert_eq!(defs_fs.len(), defs_bs.len(),
            "If slash normalization exists, both filters should return same count");
    }

    let result_fragment = dispatch_tool(&ctx, "search_definitions", &json!({
        "file": "User"
    }));
    assert!(!result_fragment.is_error);
    let output_frag: Value = serde_json::from_str(&result_fragment.content[0].text).unwrap();
    let defs_frag = output_frag["definitions"].as_array().unwrap();
    assert_eq!(defs_frag.len(), 2,
        "File filter 'User' should match both User.cs and UserService.cs, got {}", defs_frag.len());
}

/// T80 — search_reindex with invalid/non-existent directory.
#[test]
fn test_search_reindex_invalid_directory() {
    let ctx = make_empty_ctx();

    let result = dispatch_tool(&ctx, "search_reindex", &json!({
        "dir": "Z:\\nonexistent\\path\\that\\does\\not\\exist"
    }));

    assert!(result.is_error, "Reindex with non-existent dir should error");
    let error_text = &result.content[0].text;
    assert!(
        error_text.contains("Server started with") || error_text.contains("not exist") || error_text.contains("error"),
        "Error should explain the issue. Got: {}", error_text
    );
}

/// T82 — search_grep maxResults=0 semantics.
#[test]
fn test_search_grep_max_results_zero_means_unlimited() {
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
        max_response_bytes: 0,
        content_ready: Arc::new(AtomicBool::new(true)),
        def_ready: Arc::new(AtomicBool::new(true)),
    };

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

/// T43-T45 — search_find combined parameters: countOnly, maxDepth, ignoreCase, regex.
#[test]
fn test_search_find_combined_parameters() {
    use std::io::Write;
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let tmp_dir = std::env::temp_dir().join(format!("search_find_combined_{}_{}", std::process::id(), id));
    let _ = std::fs::create_dir_all(&tmp_dir);

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

    let result_depth = dispatch_tool(&ctx, "search_find", &json!({
        "pattern": ".cs",
        "maxDepth": 1,
        "ignoreCase": true
    }));
    assert!(!result_depth.is_error);
    let output_depth: Value = serde_json::from_str(&result_depth.content[0].text).unwrap();
    let depth_matches = output_depth["summary"]["totalMatches"].as_u64().unwrap();
    assert!(depth_matches < 3,
        "maxDepth=1 should find fewer than 3 files, got {}", depth_matches);

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

// ─── validate_search_dir security boundary tests ────────────────────

#[test]
fn test_validate_search_dir_subdir_accepted() {
    // Create a real temp directory structure so canonicalize works
    let base = std::env::temp_dir().join(format!("search_sec_base_{}_{}", std::process::id(),
        std::sync::atomic::AtomicU64::new(0).fetch_add(1, std::sync::atomic::Ordering::SeqCst)));
    let sub = base.join("subdir");
    std::fs::create_dir_all(&sub).unwrap();

    let result = validate_search_dir(
        &sub.to_string_lossy(),
        &base.to_string_lossy(),
    );
    assert!(result.is_ok(), "Subdirectory should be accepted, got: {:?}", result);
    assert!(result.unwrap().is_some(), "Subdirectory should return Some(canonical_subdir)");

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn test_validate_search_dir_outside_rejected() {
    // Two sibling directories — neither is a subdirectory of the other
    let parent = std::env::temp_dir().join(format!("search_sec_outside_{}", std::process::id()));
    let dir_a = parent.join("dir_a");
    let dir_b = parent.join("dir_b");
    std::fs::create_dir_all(&dir_a).unwrap();
    std::fs::create_dir_all(&dir_b).unwrap();

    let result = validate_search_dir(
        &dir_b.to_string_lossy(),
        &dir_a.to_string_lossy(),
    );
    assert!(result.is_err(), "Path outside server dir should be rejected");
    assert!(result.unwrap_err().contains("--dir"),
        "Error message should mention --dir");

    let _ = std::fs::remove_dir_all(&parent);
}

#[test]
fn test_validate_search_dir_path_traversal_rejected() {
    // Create base/subdir, then try to access base/subdir/../../.. which escapes base
    let base = std::env::temp_dir().join(format!("search_sec_traversal_{}", std::process::id()));
    let sub = base.join("subdir");
    std::fs::create_dir_all(&sub).unwrap();

    // Path traversal: subdir/../../.. resolves above base
    let traversal = sub.join("..").join("..").join("..");
    let result = validate_search_dir(
        &traversal.to_string_lossy(),
        &base.to_string_lossy(),
    );
    assert!(result.is_err(),
        "Path traversal escaping base dir should be rejected, got: {:?}", result);

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn test_validate_search_dir_windows_absolute_outside_rejected() {
    // Non-existent absolute path that clearly isn't under the server dir
    // canonicalize will fail, falling back to raw string comparison
    let result = validate_search_dir(
        r"C:\Windows\System32",
        r"C:\Repos\MyProject",
    );
    assert!(result.is_err(),
        "Absolute path outside server dir should be rejected");
}

// ─── search_find contents=true tests ─────────────────────────────────

#[test]
fn test_search_find_contents_mode() {
    use std::io::Write;
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let tmp_dir = std::env::temp_dir().join(format!("search_find_contents_{}_{}", std::process::id(), id));
    let _ = std::fs::create_dir_all(&tmp_dir);

    // Create files with distinct content
    {
        let mut f = std::fs::File::create(tmp_dir.join("alpha.txt")).unwrap();
        writeln!(f, "This file contains the magic_searchable_token here.").unwrap();
        writeln!(f, "And a second line with more content.").unwrap();
    }
    {
        let mut f = std::fs::File::create(tmp_dir.join("beta.txt")).unwrap();
        writeln!(f, "This file has completely different content.").unwrap();
        writeln!(f, "No special tokens at all.").unwrap();
    }
    {
        let mut f = std::fs::File::create(tmp_dir.join("gamma.txt")).unwrap();
        writeln!(f, "Another file that also has magic_searchable_token inside.").unwrap();
    }
    {
        let mut f = std::fs::File::create(tmp_dir.join("delta.cs")).unwrap();
        writeln!(f, "// A C# file without the search term").unwrap();
    }

    let dir_str = tmp_dir.to_string_lossy().to_string();
    let content_index = ContentIndex {
        root: dir_str.clone(), created_at: 0, max_age_secs: 3600,
        files: vec![], index: HashMap::new(), total_tokens: 0,
        extensions: vec!["txt".to_string()], file_token_counts: vec![],
        trigram: TrigramIndex::default(), trigram_dirty: false, forward: None, path_to_id: None,
    };
    let ctx = HandlerContext {
        index: Arc::new(RwLock::new(content_index)),
        def_index: None,
        server_dir: dir_str.clone(),
        server_ext: "txt".to_string(),
        metrics: false,
        index_base: tmp_dir.join(".index"),
        max_response_bytes: crate::mcp::handlers::utils::DEFAULT_MAX_RESPONSE_BYTES,
        content_ready: Arc::new(AtomicBool::new(true)),
        def_ready: Arc::new(AtomicBool::new(true)),
    };

    // Search file contents for "magic_searchable_token" in .txt files
    let result = dispatch_tool(&ctx, "search_find", &json!({
        "pattern": "magic_searchable_token",
        "contents": true,
        "ext": "txt",
        "dir": dir_str
    }));
    assert!(!result.is_error, "search_find contents=true should not error: {}", result.content[0].text);
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();

    // Should find exactly 2 files (alpha.txt and gamma.txt)
    let total_matches = output["summary"]["totalMatches"].as_u64().unwrap();
    assert_eq!(total_matches, 2, "Should find exactly 2 files containing the token, got {}", total_matches);

    // Verify the files array has 2 entries with match details
    let files = output["files"].as_array().unwrap();
    assert_eq!(files.len(), 2, "files array should have 2 entries");

    // Each matched file should have a "matches" array with line-level details
    for file_entry in files {
        assert!(file_entry["path"].is_string(), "Each result should have a path");
        let matches = file_entry["matches"].as_array().unwrap();
        assert!(!matches.is_empty(), "Each matched file should have at least one matching line");
        for m in matches {
            assert!(m["line"].is_u64(), "Each match should have a line number");
            assert!(m["text"].is_string(), "Each match should have text");
            let text = m["text"].as_str().unwrap();
            assert!(text.contains("magic_searchable_token"),
                "Matched line text should contain the search term, got: {}", text);
        }
    }

    // Verify beta.txt is NOT in results (it doesn't contain the token)
    let paths: Vec<&str> = files.iter().map(|f| f["path"].as_str().unwrap()).collect();
    for path in &paths {
        assert!(!path.contains("beta"), "beta.txt should not be in results (no match)");
        assert!(!path.contains("delta"), "delta.cs should not be in results (wrong extension)");
    }

    // Test countOnly=true with contents search
    let result_count = dispatch_tool(&ctx, "search_find", &json!({
        "pattern": "magic_searchable_token",
        "contents": true,
        "ext": "txt",
        "dir": dir_str,
        "countOnly": true
    }));
    assert!(!result_count.is_error);
    let output_count: Value = serde_json::from_str(&result_count.content[0].text).unwrap();
    assert_eq!(output_count["summary"]["totalMatches"].as_u64().unwrap(), 2,
        "countOnly should still report 2 matches");
    assert!(output_count["files"].as_array().unwrap().is_empty(),
        "countOnly=true should return empty files array");

    cleanup_tmp(&tmp_dir);
}

// ─── search_help response structure tests ────────────────────────────

#[test]
fn test_search_help_response_structure() {
    let ctx = make_empty_ctx();
    let result = dispatch_tool(&ctx, "search_help", &json!({}));
    assert!(!result.is_error, "search_help should not error");

    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();

    // Validate top-level keys exist (from tips::render_json)
    assert!(output["bestPractices"].is_array(), "Response should have 'bestPractices' array");
    assert!(output["strategyRecipes"].is_array(), "Response should have 'strategyRecipes' array");
    assert!(output["performanceTiers"].is_object(), "Response should have 'performanceTiers' object");
    assert!(output["toolPriority"].is_array(), "Response should have 'toolPriority' array");

    // bestPractices should be non-empty and each entry should have rule/why/example
    let practices = output["bestPractices"].as_array().unwrap();
    assert!(!practices.is_empty(), "bestPractices should not be empty");
    for practice in practices {
        assert!(practice["rule"].is_string(), "Each practice should have 'rule'");
        assert!(practice["why"].is_string(), "Each practice should have 'why'");
        assert!(practice["example"].is_string(), "Each practice should have 'example'");
    }

    // strategyRecipes should be non-empty and each entry should have name/when/steps/antiPatterns
    let recipes = output["strategyRecipes"].as_array().unwrap();
    assert!(!recipes.is_empty(), "strategyRecipes should not be empty");
    for recipe in recipes {
        assert!(recipe["name"].is_string(), "Each recipe should have 'name'");
        assert!(recipe["when"].is_string(), "Each recipe should have 'when'");
        assert!(recipe["steps"].is_array(), "Each recipe should have 'steps'");
        assert!(recipe["antiPatterns"].is_array(), "Each recipe should have 'antiPatterns'");
    }

    // performanceTiers should have entries
    let tiers = output["performanceTiers"].as_object().unwrap();
    assert!(!tiers.is_empty(), "performanceTiers should not be empty");

    // toolPriority should be non-empty
    let priority = output["toolPriority"].as_array().unwrap();
    assert!(!priority.is_empty(), "toolPriority should not be empty");

    // Verify counts match the source of truth
    assert_eq!(practices.len(), crate::tips::tips().len(),
        "bestPractices count should match tips::tips()");
    assert_eq!(recipes.len(), crate::tips::strategies().len(),
        "strategyRecipes count should match tips::strategies()");
    assert_eq!(priority.len(), crate::tips::tool_priority().len(),
        "toolPriority count should match tips::tool_priority()");
}

// ─── search_info response structure tests ────────────────────────────

#[test]
fn test_search_info_response_structure() {
    let ctx = make_empty_ctx();
    let result = dispatch_tool(&ctx, "search_info", &json!({}));
    assert!(!result.is_error, "search_info should not error");

    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();

    // Validate top-level keys exist (from cli::info::cmd_info_json)
    assert!(output["directory"].is_string(), "Response should have 'directory' string");
    assert!(output["indexes"].is_array(), "Response should have 'indexes' array");

    // indexes is an array (may be empty if no indexes exist, which is fine for test)
    let indexes = output["indexes"].as_array().unwrap();

    // If indexes exist, validate their structure
    for idx in indexes {
        assert!(idx["type"].is_string(), "Each index should have a 'type' field");
        let idx_type = idx["type"].as_str().unwrap();
        match idx_type {
            "file" => {
                assert!(idx["root"].is_string(), "File index should have 'root'");
                assert!(idx["entries"].is_number(), "File index should have 'entries'");
                assert!(idx["sizeMb"].is_number(), "File index should have 'sizeMb'");
                assert!(idx["ageHours"].is_number(), "File index should have 'ageHours'");
            }
            "content" => {
                assert!(idx["root"].is_string(), "Content index should have 'root'");
                assert!(idx["files"].is_number(), "Content index should have 'files'");
                assert!(idx["totalTokens"].is_number(), "Content index should have 'totalTokens'");
                assert!(idx["extensions"].is_array(), "Content index should have 'extensions'");
                assert!(idx["sizeMb"].is_number(), "Content index should have 'sizeMb'");
                assert!(idx["ageHours"].is_number(), "Content index should have 'ageHours'");
            }
            "definition" => {
                assert!(idx["root"].is_string(), "Definition index should have 'root'");
                assert!(idx["files"].is_number(), "Definition index should have 'files'");
                assert!(idx["definitions"].is_number(), "Definition index should have 'definitions'");
                assert!(idx["callSites"].is_number(), "Definition index should have 'callSites'");
                assert!(idx["extensions"].is_array(), "Definition index should have 'extensions'");
                assert!(idx["sizeMb"].is_number(), "Definition index should have 'sizeMb'");
                assert!(idx["ageHours"].is_number(), "Definition index should have 'ageHours'");
            }
            other => panic!("Unexpected index type: {}", other),
        }
    }
}