//! Tests for the definitions module — extracted from the original monolithic definitions.rs.

use super::*;
use super::parser_csharp::{parse_csharp_definitions, parse_field_signature, extract_constructor_param_types};
use std::collections::HashMap;
use std::path::PathBuf;

#[test]
fn test_definition_kind_roundtrip() {
    let kinds = vec![
        DefinitionKind::Class, DefinitionKind::Interface, DefinitionKind::Method,
        DefinitionKind::StoredProcedure, DefinitionKind::Table,
    ];
    for kind in kinds {
        let s = kind.as_str();
        let parsed: DefinitionKind = s.parse().unwrap();
        assert_eq!(parsed, kind);
    }
}

#[test]
fn test_definition_kind_display() {
    assert_eq!(format!("{}", DefinitionKind::Class), "class");
    assert_eq!(format!("{}", DefinitionKind::StoredProcedure), "storedProcedure");
    assert_eq!(format!("{}", DefinitionKind::EnumMember), "enumMember");
}

#[test]
fn test_definition_kind_parse_invalid() {
    let result = "invalid_kind".parse::<DefinitionKind>();
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Unknown definition kind"));
}

#[test]
fn test_definition_kind_parse_case_insensitive() {
    let parsed: DefinitionKind = "CLASS".parse().unwrap();
    assert_eq!(parsed, DefinitionKind::Class);
    let parsed: DefinitionKind = "StoredProcedure".parse().unwrap();
    assert_eq!(parsed, DefinitionKind::StoredProcedure);
}

#[test]
fn test_definition_kind_roundtrip_all_variants() {
    let all_kinds = vec![
        DefinitionKind::Class, DefinitionKind::Interface, DefinitionKind::Enum,
        DefinitionKind::Struct, DefinitionKind::Record, DefinitionKind::Method,
        DefinitionKind::Property, DefinitionKind::Field, DefinitionKind::Constructor,
        DefinitionKind::Delegate, DefinitionKind::Event, DefinitionKind::EnumMember,
        DefinitionKind::StoredProcedure, DefinitionKind::Table, DefinitionKind::View,
        DefinitionKind::SqlFunction, DefinitionKind::UserDefinedType,
        DefinitionKind::Column, DefinitionKind::SqlIndex,
    ];
    for kind in all_kinds {
        let s = kind.to_string();
        let parsed: DefinitionKind = s.parse().unwrap_or_else(|e| panic!("Failed to parse '{}': {}", s, e));
        assert_eq!(parsed, kind, "Roundtrip failed for {:?} -> '{}' -> {:?}", kind, s, parsed);
    }
}

#[test]
fn test_parse_csharp_class() {
    let source = r#"
using System;

namespace MyApp
{
    [ServiceProvider(typeof(IMyService))]
    public sealed class MyService : BaseService, IMyService
    {
        [ServiceDependency]
        private readonly ILogger m_logger = null;

        public string Name { get; set; }

        public async Task<Result> DoWork(string input, int count)
        {
            return null;
        }

        public MyService(ILogger logger)
        {
        }
    }

    public interface IMyService
    {
        Task<Result> DoWork(string input, int count);
    }

    public enum Status
    {
        Active,
        Inactive,
        Deleted
    }
}
"#;

    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_c_sharp::LANGUAGE.into()).unwrap();

    let (defs, _call_sites) = parse_csharp_definitions(&mut parser, source, 0);

    let class_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Class).collect();
    assert_eq!(class_defs.len(), 1);
    assert_eq!(class_defs[0].name, "MyService");
    assert!(!class_defs[0].attributes.is_empty());
    assert!(class_defs[0].base_types.len() >= 1);

    let iface_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Interface).collect();
    assert_eq!(iface_defs.len(), 1);
    assert_eq!(iface_defs[0].name, "IMyService");

    let method_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Method).collect();
    assert!(method_defs.len() >= 1);
    let do_work = method_defs.iter().find(|d| d.name == "DoWork");
    assert!(do_work.is_some());
    assert_eq!(do_work.unwrap().parent, Some("MyService".to_string()));

    let prop_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Property).collect();
    assert!(prop_defs.len() >= 1);
    assert_eq!(prop_defs[0].name, "Name");

    let field_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Field).collect();
    assert!(field_defs.len() >= 1);

    let ctor_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Constructor).collect();
    assert_eq!(ctor_defs.len(), 1);
    assert_eq!(ctor_defs[0].name, "MyService");

    let enum_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Enum).collect();
    assert_eq!(enum_defs.len(), 1);
    assert_eq!(enum_defs[0].name, "Status");

    let member_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::EnumMember).collect();
    assert_eq!(member_defs.len(), 3);
}

#[test]
fn test_definition_index_build_and_search() {
    let dir = std::env::temp_dir().join("search_defindex_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("test.cs"), "public class TestClass : BaseClass { public void TestMethod() {} }").unwrap();
    std::fs::write(dir.join("test.sql"), "CREATE TABLE TestTable (Id INT NOT NULL)").unwrap();

    let args = DefIndexArgs { dir: dir.to_string_lossy().to_string(), ext: "cs,sql".to_string(), threads: 1 };
    let index = build_definition_index(&args);

    assert_eq!(index.files.len(), 2);
    assert!(!index.definitions.is_empty());
    assert!(index.name_index.contains_key("testclass"));
    assert!(index.name_index.contains_key("testmethod"));
    assert!(index.kind_index.contains_key(&DefinitionKind::Class));
    assert!(index.kind_index.contains_key(&DefinitionKind::Method));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_attribute_index_no_duplicates_for_same_attr_name() {
    let dir = std::env::temp_dir().join("search_attr_dedup_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("service.cs"), r#"
[Obsolete]
[Obsolete("Use NewService instead")]
public class MyService { }

[Obsolete]
public class OtherService { }
"#).unwrap();

    let args = DefIndexArgs { dir: dir.to_string_lossy().to_string(), ext: "cs".to_string(), threads: 1 };
    let index = build_definition_index(&args);

    let attr_indices = index.attribute_index.get("obsolete").expect("should have 'obsolete'");
    let mut sorted = attr_indices.clone();
    sorted.sort();
    let deduped_len = { let mut d = sorted.clone(); d.dedup(); d.len() };
    assert_eq!(attr_indices.len(), deduped_len);
    assert_eq!(attr_indices.len(), 2);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_definition_index_serialization() {
    let index = DefinitionIndex {
        root: ".".to_string(), created_at: 1000, extensions: vec!["cs".to_string()],
        files: vec!["test.cs".to_string()],
        definitions: vec![DefinitionEntry {
            file_id: 0, name: "TestClass".to_string(), kind: DefinitionKind::Class,
            line_start: 1, line_end: 10, parent: None,
            signature: Some("public class TestClass".to_string()),
            modifiers: vec!["public".to_string()], attributes: Vec::new(), base_types: Vec::new(),
        }],
        name_index: { let mut m = HashMap::new(); m.insert("testclass".to_string(), vec![0]); m },
        kind_index: { let mut m = HashMap::new(); m.insert(DefinitionKind::Class, vec![0]); m },
        attribute_index: HashMap::new(), base_type_index: HashMap::new(),
        file_index: { let mut m = HashMap::new(); m.insert(0, vec![0]); m },
        path_to_id: HashMap::new(), method_calls: HashMap::new(), parse_errors: 0, lossy_file_count: 0, empty_file_ids: Vec::new(),
    };

    let encoded = bincode::serialize(&index).unwrap();
    let decoded: DefinitionIndex = bincode::deserialize(&encoded).unwrap();
    assert_eq!(decoded.definitions.len(), 1);
    assert_eq!(decoded.definitions[0].name, "TestClass");
    assert_eq!(decoded.definitions[0].kind, DefinitionKind::Class);
}

#[test]
fn test_incremental_update_new_file() {
    let dir = std::env::temp_dir().join("search_def_incr_new");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let test_file = dir.join("new.cs");
    std::fs::write(&test_file, "public class NewClass { public void NewMethod() {} }").unwrap();

    let mut index = DefinitionIndex {
        root: ".".to_string(), created_at: 0, extensions: vec!["cs".to_string()],
        files: Vec::new(), definitions: Vec::new(), name_index: HashMap::new(),
        kind_index: HashMap::new(), attribute_index: HashMap::new(),
        base_type_index: HashMap::new(), file_index: HashMap::new(),
        path_to_id: HashMap::new(), method_calls: HashMap::new(), parse_errors: 0, lossy_file_count: 0, empty_file_ids: Vec::new(),
    };

    let clean = PathBuf::from(crate::clean_path(&test_file.to_string_lossy()));
    update_file_definitions(&mut index, &clean);

    assert!(!index.definitions.is_empty());
    assert!(index.name_index.contains_key("newclass"));
    assert!(index.name_index.contains_key("newmethod"));
    assert_eq!(index.files.len(), 1);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_incremental_update_existing_file() {
    let dir = std::env::temp_dir().join("search_def_incr_update");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let test_file = dir.join("existing.cs");
    std::fs::write(&test_file, "public class OldClass { }").unwrap();

    let clean = PathBuf::from(crate::clean_path(&test_file.to_string_lossy()));

    let mut index = DefinitionIndex {
        root: ".".to_string(), created_at: 0, extensions: vec!["cs".to_string()],
        files: vec![clean.to_string_lossy().to_string()],
        definitions: vec![DefinitionEntry {
            file_id: 0, name: "OldClass".to_string(), kind: DefinitionKind::Class,
            line_start: 1, line_end: 1, parent: None, signature: None,
            modifiers: Vec::new(), attributes: Vec::new(), base_types: Vec::new(),
        }],
        name_index: { let mut m = HashMap::new(); m.insert("oldclass".to_string(), vec![0]); m },
        kind_index: { let mut m = HashMap::new(); m.insert(DefinitionKind::Class, vec![0]); m },
        attribute_index: HashMap::new(), base_type_index: HashMap::new(),
        file_index: { let mut m = HashMap::new(); m.insert(0, vec![0]); m },
        path_to_id: { let mut m = HashMap::new(); m.insert(clean.clone(), 0u32); m },
        method_calls: HashMap::new(), parse_errors: 0, lossy_file_count: 0, empty_file_ids: Vec::new(),
    };

    std::fs::write(&test_file, "public class UpdatedClass { public int Value { get; set; } }").unwrap();
    update_file_definitions(&mut index, &clean);

    assert!(!index.name_index.contains_key("oldclass"));
    assert!(index.name_index.contains_key("updatedclass"));
    assert!(index.name_index.contains_key("value"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_remove_file_from_def_index() {
    let mut index = DefinitionIndex {
        root: ".".to_string(), created_at: 0, extensions: vec!["cs".to_string()],
        files: vec!["file0.cs".to_string(), "file1.cs".to_string()],
        definitions: vec![
            DefinitionEntry { file_id: 0, name: "ClassA".to_string(), kind: DefinitionKind::Class, line_start: 1, line_end: 10, parent: None, signature: None, modifiers: Vec::new(), attributes: Vec::new(), base_types: Vec::new() },
            DefinitionEntry { file_id: 1, name: "ClassB".to_string(), kind: DefinitionKind::Class, line_start: 1, line_end: 10, parent: None, signature: None, modifiers: Vec::new(), attributes: Vec::new(), base_types: Vec::new() },
        ],
        name_index: { let mut m = HashMap::new(); m.insert("classa".to_string(), vec![0]); m.insert("classb".to_string(), vec![1]); m },
        kind_index: { let mut m = HashMap::new(); m.insert(DefinitionKind::Class, vec![0, 1]); m },
        attribute_index: HashMap::new(), base_type_index: HashMap::new(),
        file_index: { let mut m = HashMap::new(); m.insert(0, vec![0]); m.insert(1, vec![1]); m },
        path_to_id: { let mut m = HashMap::new(); m.insert(PathBuf::from("file0.cs"), 0); m.insert(PathBuf::from("file1.cs"), 1); m },
        method_calls: HashMap::new(), parse_errors: 0, lossy_file_count: 0, empty_file_ids: Vec::new(),
    };

    remove_file_from_def_index(&mut index, &PathBuf::from("file0.cs"));
    assert!(!index.name_index.contains_key("classa"));
    assert!(index.name_index.contains_key("classb"));
    assert!(!index.path_to_id.contains_key(&PathBuf::from("file0.cs")));
    assert!(index.path_to_id.contains_key(&PathBuf::from("file1.cs")));
}

// ─── Call Site Extraction Tests ──────────────────────────────────

#[test] fn test_call_site_extraction_simple_calls() {
    let source = r#"
public class OrderService { public void Process() { Validate(); SendEmail(); } }
"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_c_sharp::LANGUAGE.into()).unwrap();
    let (defs, call_sites) = parse_csharp_definitions(&mut parser, source, 0);
    let pi = defs.iter().position(|d| d.name == "Process").unwrap();
    let pc: Vec<_> = call_sites.iter().filter(|(i, _)| *i == pi).collect();
    assert!(!pc.is_empty());
    let names: Vec<&str> = pc[0].1.iter().map(|c| c.method_name.as_str()).collect();
    assert!(names.contains(&"Validate"));
    assert!(names.contains(&"SendEmail"));
}

#[test] fn test_call_site_extraction_field_access() {
    let source = r#"
public class OrderService {
    private readonly IUserService _userService;
    private readonly ILogger _logger;
    public OrderService(IUserService userService, ILogger logger) { _userService = userService; _logger = logger; }
    public void Process(int id) { var user = _userService.GetUser(id); _logger.LogInfo("done"); }
}
"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_c_sharp::LANGUAGE.into()).unwrap();
    let (defs, cs) = parse_csharp_definitions(&mut parser, source, 0);
    let pi = defs.iter().position(|d| d.name == "Process").unwrap();
    let pc: Vec<_> = cs.iter().filter(|(i, _)| *i == pi).collect();
    assert!(!pc.is_empty());
    let gu = pc[0].1.iter().find(|c| c.method_name == "GetUser");
    assert!(gu.is_some());
    assert_eq!(gu.unwrap().receiver_type.as_deref(), Some("IUserService"));
    let li = pc[0].1.iter().find(|c| c.method_name == "LogInfo");
    assert!(li.is_some());
    assert_eq!(li.unwrap().receiver_type.as_deref(), Some("ILogger"));
}

#[test] fn test_call_site_extraction_constructor_param_di() {
    let source = r#"
public class OrderService {
    private readonly IOrderRepository _orderRepo;
    public OrderService(IOrderRepository orderRepo) { _orderRepo = orderRepo; }
    public void Save() { _orderRepo.Insert(); }
}
"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_c_sharp::LANGUAGE.into()).unwrap();
    let (defs, cs) = parse_csharp_definitions(&mut parser, source, 0);
    let si = defs.iter().position(|d| d.name == "Save").unwrap();
    let sc: Vec<_> = cs.iter().filter(|(i, _)| *i == si).collect();
    assert!(!sc.is_empty());
    let ins = sc[0].1.iter().find(|c| c.method_name == "Insert");
    assert!(ins.is_some());
    assert_eq!(ins.unwrap().receiver_type.as_deref(), Some("IOrderRepository"));
}

#[test] fn test_call_site_extraction_object_creation() {
    let source = r#"
public class Factory { public void Create() { var obj = new OrderValidator(); } }
"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_c_sharp::LANGUAGE.into()).unwrap();
    let (defs, cs) = parse_csharp_definitions(&mut parser, source, 0);
    let ci = defs.iter().position(|d| d.name == "Create").unwrap();
    let cc: Vec<_> = cs.iter().filter(|(i, _)| *i == ci).collect();
    assert!(!cc.is_empty());
    let nc = cc[0].1.iter().find(|c| c.method_name == "OrderValidator");
    assert!(nc.is_some());
    assert_eq!(nc.unwrap().receiver_type.as_deref(), Some("OrderValidator"));
}

#[test] fn test_call_site_extraction_this_and_static() {
    let source = r#"
public class MyClass {
    public void Method1() { this.Method2(); Helper.DoWork(); }
    public void Method2() {}
}
"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_c_sharp::LANGUAGE.into()).unwrap();
    let (defs, cs) = parse_csharp_definitions(&mut parser, source, 0);
    let mi = defs.iter().position(|d| d.name == "Method1").unwrap();
    let mc: Vec<_> = cs.iter().filter(|(i, _)| *i == mi).collect();
    assert!(!mc.is_empty());
    let m2 = mc[0].1.iter().find(|c| c.method_name == "Method2");
    assert!(m2.is_some());
    assert_eq!(m2.unwrap().receiver_type.as_deref(), Some("MyClass"));
    let dw = mc[0].1.iter().find(|c| c.method_name == "DoWork");
    assert!(dw.is_some());
    assert_eq!(dw.unwrap().receiver_type.as_deref(), Some("Helper"));
}

#[test] fn test_parse_field_signature() {
    assert_eq!(parse_field_signature("IUserService _userService"), Some(("IUserService".to_string(), "_userService".to_string())));
    assert_eq!(parse_field_signature("ILogger<OrderService> _logger"), Some(("ILogger".to_string(), "_logger".to_string())));
    assert_eq!(parse_field_signature("string Name"), Some(("string".to_string(), "Name".to_string())));
}

#[test] fn test_extract_constructor_param_types() {
    let sig = "public OrderService(IUserService userService, ILogger<OrderService> logger)";
    let params = extract_constructor_param_types(sig);
    assert_eq!(params.len(), 2);
    assert_eq!(params[0], ("userService".to_string(), "IUserService".to_string()));
    assert_eq!(params[1], ("logger".to_string(), "ILogger".to_string()));
}

#[test] fn test_call_site_no_calls_for_empty_method() {
    let source = r#"public class Empty { public void Nothing() {} }"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_c_sharp::LANGUAGE.into()).unwrap();
    let (defs, cs) = parse_csharp_definitions(&mut parser, source, 0);
    let ni = defs.iter().position(|d| d.name == "Nothing").unwrap();
    let nc: Vec<_> = cs.iter().filter(|(i, _)| *i == ni).collect();
    assert!(nc.is_empty());
}

#[test] fn test_implicit_this_call_extraction() {
    let source = r#"
public class OrderService {
    public async Task ProcessAsync() { ValidateAsync(); await SaveAsync(); }
    public Task ValidateAsync() => Task.CompletedTask;
    public Task SaveAsync() => Task.CompletedTask;
}
"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_c_sharp::LANGUAGE.into()).unwrap();
    let (defs, cs) = parse_csharp_definitions(&mut parser, source, 0);
    let pi = defs.iter().position(|d| d.name == "ProcessAsync").unwrap();
    let pc: Vec<_> = cs.iter().filter(|(i, _)| *i == pi).collect();
    assert!(!pc.is_empty());
    let names: Vec<&str> = pc[0].1.iter().map(|c| c.method_name.as_str()).collect();
    assert!(names.contains(&"ValidateAsync"));
    assert!(names.contains(&"SaveAsync"));
}

#[test] fn test_call_sites_chained_calls() {
    let source = r#"
public class Processor {
    private readonly IQueryBuilder _builder;
    public void Run() { _builder.Where("x > 1").OrderBy("x").ToList(); }
}
"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_c_sharp::LANGUAGE.into()).unwrap();
    let (defs, cs) = parse_csharp_definitions(&mut parser, source, 0);
    let ri = defs.iter().position(|d| d.name == "Run").unwrap();
    let rc: Vec<_> = cs.iter().filter(|(i, _)| *i == ri).collect();
    assert!(!rc.is_empty());
    let names: Vec<&str> = rc[0].1.iter().map(|c| c.method_name.as_str()).collect();
    assert!(names.contains(&"ToList"));
}

#[test] fn test_call_sites_lambda() {
    let source = r#"
public class DataProcessor {
    public void Transform(List<Item> items) { items.ForEach(x => ProcessAsync(x)); }
    private Task<Item> ProcessAsync(Item x) => Task.FromResult(x);
}
"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_c_sharp::LANGUAGE.into()).unwrap();
    let (defs, cs) = parse_csharp_definitions(&mut parser, source, 0);
    let ti = defs.iter().position(|d| d.name == "Transform").unwrap();
    let tc: Vec<_> = cs.iter().filter(|(i, _)| *i == ti).collect();
    assert!(!tc.is_empty());
    let names: Vec<&str> = tc[0].1.iter().map(|c| c.method_name.as_str()).collect();
    assert!(names.contains(&"ForEach"));
    assert!(names.contains(&"ProcessAsync"));
}

#[test] fn test_field_type_resolution_with_generics() {
    let source = r#"
public class OrderService {
    private readonly ILogger<OrderService> _logger;
    public void Process() { _logger.LogInformation("processing"); }
}
"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_c_sharp::LANGUAGE.into()).unwrap();
    let (defs, cs) = parse_csharp_definitions(&mut parser, source, 0);
    let pi = defs.iter().position(|d| d.name == "Process").unwrap();
    let pc: Vec<_> = cs.iter().filter(|(i, _)| *i == pi).collect();
    assert!(!pc.is_empty());
    let lc = pc[0].1.iter().find(|c| c.method_name == "LogInformation");
    assert!(lc.is_some());
    assert_eq!(lc.unwrap().receiver_type.as_deref(), Some("ILogger"));
}

#[test] fn test_incremental_update_preserves_call_graph() {
    let dir = std::env::temp_dir().join("search_def_incr_callgraph");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let test_file = dir.join("service.cs");
    std::fs::write(&test_file, r#"
public class MyService {
    private readonly IRepo _repo;
    public void Save() { _repo.Insert(); }
}
"#).unwrap();

    let mut index = DefinitionIndex {
        root: ".".to_string(), created_at: 0, extensions: vec!["cs".to_string()],
        files: Vec::new(), definitions: Vec::new(), name_index: HashMap::new(),
        kind_index: HashMap::new(), attribute_index: HashMap::new(),
        base_type_index: HashMap::new(), file_index: HashMap::new(),
        path_to_id: HashMap::new(), method_calls: HashMap::new(), parse_errors: 0, lossy_file_count: 0, empty_file_ids: Vec::new(),
    };

    let clean = PathBuf::from(crate::clean_path(&test_file.to_string_lossy()));
    update_file_definitions(&mut index, &clean);
    assert!(!index.method_calls.is_empty());

    let save_idx = index.definitions.iter().position(|d| d.name == "Save").unwrap() as u32;
    let save_calls = index.method_calls.get(&save_idx);
    assert!(save_calls.is_some());
    assert!(save_calls.unwrap().iter().any(|c| c.method_name == "Insert"));

    std::fs::write(&test_file, r#"
public class MyService {
    private readonly IRepo _repo;
    public void Save() { _repo.Update(); _repo.Commit(); }
}
"#).unwrap();

    update_file_definitions(&mut index, &clean);

    let new_save_idx = index.definitions.iter().enumerate()
        .rfind(|(_, d)| d.name == "Save")
        .map(|(i, _)| i as u32)
        .unwrap();
    let new_calls = index.method_calls.get(&new_save_idx);
    assert!(new_calls.is_some());
    let new_names: Vec<&str> = new_calls.unwrap().iter().map(|c| c.method_name.as_str()).collect();
    assert!(new_names.contains(&"Update"));
    assert!(new_names.contains(&"Commit"));
    assert!(!new_names.contains(&"Insert"));

    let _ = std::fs::remove_dir_all(&dir);
}

// ─── TypeScript Parsing Tests ────────────────────────────────────────

use super::parser_typescript::parse_typescript_definitions;

#[test]
fn test_parse_ts_class() {
    let source = "export class UserService extends BaseService implements IUserService { }";
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, _call_sites) = parse_typescript_definitions(&mut parser, source, 0);

    let class_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Class).collect();
    assert_eq!(class_defs.len(), 1);
    assert_eq!(class_defs[0].name, "UserService");
    assert!(class_defs[0].base_types.iter().any(|b| b.contains("BaseService")));
    assert!(class_defs[0].base_types.iter().any(|b| b.contains("IUserService")));
    assert!(class_defs[0].modifiers.contains(&"export".to_string()));
}

#[test]
fn test_parse_ts_abstract_class() {
    let source = r#"abstract class AbstractHandler {
    abstract handle(): void;
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, _call_sites) = parse_typescript_definitions(&mut parser, source, 0);

    let class_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Class).collect();
    assert_eq!(class_defs.len(), 1);
    assert_eq!(class_defs[0].name, "AbstractHandler");
    assert!(class_defs[0].modifiers.contains(&"abstract".to_string()));

    let method_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Method).collect();
    assert!(method_defs.len() >= 1);
    assert_eq!(method_defs[0].name, "handle");
    assert!(method_defs[0].modifiers.contains(&"abstract".to_string()));
}

#[test]
fn test_parse_ts_interface() {
    let source = r#"export interface IOrderProcessor {
    process(order: Order): Promise<void>;
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, _call_sites) = parse_typescript_definitions(&mut parser, source, 0);

    let iface_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Interface).collect();
    assert_eq!(iface_defs.len(), 1);
    assert_eq!(iface_defs[0].name, "IOrderProcessor");
    assert!(iface_defs[0].modifiers.contains(&"export".to_string()));

    // Interface should have a property child for the method signature
    let prop_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Property).collect();
    assert!(prop_defs.len() >= 1);
}

#[test]
fn test_parse_ts_function() {
    let source = "export async function fetchUser(id: string): Promise<User> { return {} as User; }";
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, _call_sites) = parse_typescript_definitions(&mut parser, source, 0);

    let fn_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Function).collect();
    assert_eq!(fn_defs.len(), 1);
    assert_eq!(fn_defs[0].name, "fetchUser");
    assert!(fn_defs[0].modifiers.contains(&"export".to_string()));
    assert!(fn_defs[0].modifiers.contains(&"async".to_string()));
    assert!(fn_defs[0].signature.is_some());
    let sig = fn_defs[0].signature.as_ref().unwrap();
    assert!(sig.contains("id: string"));
}

#[test]
fn test_parse_ts_method() {
    let source = r#"class UserManager {
    public async getUser(id: string): Promise<User> { return {} as User; }
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, _call_sites) = parse_typescript_definitions(&mut parser, source, 0);

    let method_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Method).collect();
    assert_eq!(method_defs.len(), 1);
    assert_eq!(method_defs[0].name, "getUser");
    assert!(method_defs[0].modifiers.contains(&"public".to_string()));
    assert!(method_defs[0].modifiers.contains(&"async".to_string()));
    assert_eq!(method_defs[0].parent, Some("UserManager".to_string()));
}

#[test]
fn test_parse_ts_constructor() {
    let source = r#"class OrderService {
    constructor(private userService: IUserService) { }
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, _call_sites) = parse_typescript_definitions(&mut parser, source, 0);

    let ctor_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Constructor).collect();
    assert_eq!(ctor_defs.len(), 1);
    assert_eq!(ctor_defs[0].name, "constructor");
    assert_eq!(ctor_defs[0].parent, Some("OrderService".to_string()));
    assert!(ctor_defs[0].signature.is_some());
    let sig = ctor_defs[0].signature.as_ref().unwrap();
    assert!(sig.contains("userService"));
}

#[test]
fn test_parse_ts_enum() {
    let source = r#"export enum OrderStatus {
    Pending,
    Active,
    Completed
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, _call_sites) = parse_typescript_definitions(&mut parser, source, 0);

    let enum_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Enum).collect();
    assert_eq!(enum_defs.len(), 1);
    assert_eq!(enum_defs[0].name, "OrderStatus");

    let member_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::EnumMember).collect();
    assert_eq!(member_defs.len(), 3);
}

#[test]
fn test_parse_ts_type_alias() {
    let source = "export type UserId = string | number;";
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, _call_sites) = parse_typescript_definitions(&mut parser, source, 0);

    let ta_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::TypeAlias).collect();
    assert_eq!(ta_defs.len(), 1);
    assert_eq!(ta_defs[0].name, "UserId");
    assert!(ta_defs[0].modifiers.contains(&"export".to_string()));
}

#[test]
fn test_parse_ts_variable() {
    let source = "export const MAX_RETRIES = 3;";
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, _call_sites) = parse_typescript_definitions(&mut parser, source, 0);

    let var_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Variable).collect();
    assert_eq!(var_defs.len(), 1);
    assert_eq!(var_defs[0].name, "MAX_RETRIES");
    assert!(var_defs[0].modifiers.contains(&"export".to_string()));
}

#[test]
fn test_parse_ts_decorators() {
    let source = r#"@Injectable()
@Component({selector: 'app'})
class AppComponent {}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, _call_sites) = parse_typescript_definitions(&mut parser, source, 0);

    let class_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Class).collect();
    assert_eq!(class_defs.len(), 1);
    assert_eq!(class_defs[0].name, "AppComponent");
    assert_eq!(class_defs[0].attributes.len(), 2);
    assert!(class_defs[0].attributes.iter().any(|a| a.contains("Injectable")));
    assert!(class_defs[0].attributes.iter().any(|a| a.contains("Component")));
}

#[test]
fn test_parse_ts_field() {
    let source = r#"class DataHolder {
    private readonly name: string = '';
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, _call_sites) = parse_typescript_definitions(&mut parser, source, 0);

    let field_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Field).collect();
    assert_eq!(field_defs.len(), 1);
    assert_eq!(field_defs[0].name, "name");
    assert!(field_defs[0].modifiers.contains(&"private".to_string()));
    assert!(field_defs[0].modifiers.contains(&"readonly".to_string()));
}

#[test]
fn test_parse_ts_interface_property() {
    let source = r#"interface IEntity {
    readonly id: string;
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, _call_sites) = parse_typescript_definitions(&mut parser, source, 0);

    let prop_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Property).collect();
    assert_eq!(prop_defs.len(), 1);
    assert_eq!(prop_defs[0].name, "id");
    assert!(prop_defs[0].modifiers.contains(&"readonly".to_string()));
}

#[test]
fn test_parse_tsx_file() {
    let source = r#"export class AppComponent {
    render() { return <div/>; }
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TSX.into()).unwrap();
    let (defs, _call_sites) = parse_typescript_definitions(&mut parser, source, 0);

    let class_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Class).collect();
    assert_eq!(class_defs.len(), 1);
    assert_eq!(class_defs[0].name, "AppComponent");

    let method_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Method).collect();
    assert_eq!(method_defs.len(), 1);
    assert_eq!(method_defs[0].name, "render");
}

#[test]
fn test_ts_incremental_update() {
    let dir = std::env::temp_dir().join("search_def_ts_incr");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // Step 1: Create a .ts file and add it to the index
    let test_file = dir.join("service.ts");
    std::fs::write(&test_file, "export class OrderService { process(): void {} }").unwrap();

    let mut index = DefinitionIndex {
        root: ".".to_string(), created_at: 0, extensions: vec!["ts".to_string()],
        files: Vec::new(), definitions: Vec::new(), name_index: HashMap::new(),
        kind_index: HashMap::new(), attribute_index: HashMap::new(),
        base_type_index: HashMap::new(), file_index: HashMap::new(),
        path_to_id: HashMap::new(), method_calls: HashMap::new(), parse_errors: 0, lossy_file_count: 0, empty_file_ids: Vec::new(),
    };

    let clean = PathBuf::from(crate::clean_path(&test_file.to_string_lossy()));
    update_file_definitions(&mut index, &clean);

    assert!(!index.definitions.is_empty());
    assert!(index.name_index.contains_key("orderservice"));
    assert!(index.name_index.contains_key("process"));
    assert_eq!(index.files.len(), 1);

    // Step 2: Modify the .ts file — rename class, add a method
    std::fs::write(&test_file, r#"export class UpdatedService {
    execute(): void {}
    validate(): boolean { return true; }
}"#).unwrap();

    update_file_definitions(&mut index, &clean);

    assert!(!index.name_index.contains_key("orderservice"));
    assert!(!index.name_index.contains_key("process"));
    assert!(index.name_index.contains_key("updatedservice"));
    assert!(index.name_index.contains_key("execute"));
    assert!(index.name_index.contains_key("validate"));

    // Step 3: Remove the file (simulate deletion by writing empty)
    std::fs::write(&test_file, "").unwrap();
    update_file_definitions(&mut index, &clean);

    // All named definitions from that file should be gone from name index
    assert!(!index.name_index.contains_key("updatedservice"));
    assert!(!index.name_index.contains_key("execute"));
    assert!(!index.name_index.contains_key("validate"));

    let _ = std::fs::remove_dir_all(&dir);
}


// ─── TypeScript Call-Site Extraction Tests ────────────────────────────

#[test]
fn test_ts_this_method_call() {
    let source = r#"class OrderService {
    process(): void {
        this.doSomething();
    }
    doSomething(): void {}
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites) = parse_typescript_definitions(&mut parser, source, 0);

    let pi = defs.iter().position(|d| d.name == "process").unwrap();
    let pc: Vec<_> = call_sites.iter().filter(|(i, _)| *i == pi).collect();
    assert!(!pc.is_empty(), "Expected call sites for 'process' method");
    let ds = pc[0].1.iter().find(|c| c.method_name == "doSomething");
    assert!(ds.is_some(), "Expected call to 'doSomething'");
    assert_eq!(ds.unwrap().receiver_type.as_deref(), Some("OrderService"));
}

#[test]
fn test_ts_this_field_method_call() {
    let source = r#"class OrderController {
    constructor(private userService: UserService) {}
    handle(): void {
        this.userService.getUser();
    }
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites) = parse_typescript_definitions(&mut parser, source, 0);

    let hi = defs.iter().position(|d| d.name == "handle").unwrap();
    let hc: Vec<_> = call_sites.iter().filter(|(i, _)| *i == hi).collect();
    assert!(!hc.is_empty(), "Expected call sites for 'handle' method");
    let gu = hc[0].1.iter().find(|c| c.method_name == "getUser");
    assert!(gu.is_some(), "Expected call to 'getUser'");
    assert_eq!(gu.unwrap().receiver_type.as_deref(), Some("UserService"));
}

#[test]
fn test_ts_standalone_function_call() {
    let source = r#"function processOrder(): void {
    someHelper();
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites) = parse_typescript_definitions(&mut parser, source, 0);

    let pi = defs.iter().position(|d| d.name == "processOrder").unwrap();
    let pc: Vec<_> = call_sites.iter().filter(|(i, _)| *i == pi).collect();
    assert!(!pc.is_empty(), "Expected call sites for 'processOrder'");
    let sh = pc[0].1.iter().find(|c| c.method_name == "someHelper");
    assert!(sh.is_some(), "Expected call to 'someHelper'");
    assert_eq!(sh.unwrap().receiver_type, None);
}

#[test]
fn test_ts_new_expression() {
    let source = r#"class Factory {
    create(): void {
        const svc = new UserService();
    }
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites) = parse_typescript_definitions(&mut parser, source, 0);

    let ci = defs.iter().position(|d| d.name == "create").unwrap();
    let cc: Vec<_> = call_sites.iter().filter(|(i, _)| *i == ci).collect();
    assert!(!cc.is_empty(), "Expected call sites for 'create'");
    let nc = cc[0].1.iter().find(|c| c.method_name == "UserService");
    assert!(nc.is_some(), "Expected new UserService call");
    assert_eq!(nc.unwrap().receiver_type.as_deref(), Some("UserService"));
}

#[test]
fn test_ts_static_method_call() {
    let source = r#"class Processor {
    run(): void {
        MathUtils.calculate();
    }
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites) = parse_typescript_definitions(&mut parser, source, 0);

    let ri = defs.iter().position(|d| d.name == "run").unwrap();
    let rc: Vec<_> = call_sites.iter().filter(|(i, _)| *i == ri).collect();
    assert!(!rc.is_empty(), "Expected call sites for 'run'");
    let mc = rc[0].1.iter().find(|c| c.method_name == "calculate");
    assert!(mc.is_some(), "Expected call to 'calculate'");
    assert_eq!(mc.unwrap().receiver_type.as_deref(), Some("MathUtils"));
}

#[test]
fn test_ts_arrow_function_class_property() {
    let source = r#"class ItemProcessor {
    processItem = (item: string): void => {
        this.validate(item);
    };
    validate(item: string): void {}
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites) = parse_typescript_definitions(&mut parser, source, 0);

    let pi = defs.iter().position(|d| d.name == "processItem").unwrap();
    let pc: Vec<_> = call_sites.iter().filter(|(i, _)| *i == pi).collect();
    assert!(!pc.is_empty(), "Expected call sites for 'processItem' arrow function");
    let vc = pc[0].1.iter().find(|c| c.method_name == "validate");
    assert!(vc.is_some(), "Expected call to 'validate'");
    assert_eq!(vc.unwrap().receiver_type.as_deref(), Some("ItemProcessor"));
}

#[test]
fn test_ts_constructor_di_field_types() {
    let source = r#"class OrderHandler {
    constructor(private orderRepo: OrderRepository, private logger: Logger) {}
    execute(): void {
        this.orderRepo.save();
        this.logger.info("done");
    }
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites) = parse_typescript_definitions(&mut parser, source, 0);

    let ei = defs.iter().position(|d| d.name == "execute").unwrap();
    let ec: Vec<_> = call_sites.iter().filter(|(i, _)| *i == ei).collect();
    assert!(!ec.is_empty(), "Expected call sites for 'execute'");

    let save = ec[0].1.iter().find(|c| c.method_name == "save");
    assert!(save.is_some(), "Expected call to 'save'");
    assert_eq!(save.unwrap().receiver_type.as_deref(), Some("OrderRepository"));

    let info = ec[0].1.iter().find(|c| c.method_name == "info");
    assert!(info.is_some(), "Expected call to 'info'");
    assert_eq!(info.unwrap().receiver_type.as_deref(), Some("Logger"));
}

#[test]
fn test_ts_multiple_calls_in_method() {
    let source = r#"class DataService {
    constructor(private repo: DataRepository) {}
    process(): void {
        this.validate();
        this.repo.findAll();
        const result = new ResultSet();
        helperFn();
        Formatter.format();
    }
    validate(): void {}
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites) = parse_typescript_definitions(&mut parser, source, 0);

    let pi = defs.iter().position(|d| d.name == "process").unwrap();
    let pc: Vec<_> = call_sites.iter().filter(|(i, _)| *i == pi).collect();
    assert!(!pc.is_empty(), "Expected call sites for 'process'");

    let names: Vec<&str> = pc[0].1.iter().map(|c| c.method_name.as_str()).collect();
    assert!(names.contains(&"validate"), "Expected call to 'validate'");
    assert!(names.contains(&"findAll"), "Expected call to 'findAll'");
    assert!(names.contains(&"ResultSet"), "Expected new ResultSet");
    assert!(names.contains(&"helperFn"), "Expected call to 'helperFn'");
    assert!(names.contains(&"format"), "Expected call to 'format'");

    // Check receiver types
    let validate_call = pc[0].1.iter().find(|c| c.method_name == "validate").unwrap();
    assert_eq!(validate_call.receiver_type.as_deref(), Some("DataService"));
    let find_call = pc[0].1.iter().find(|c| c.method_name == "findAll").unwrap();
    assert_eq!(find_call.receiver_type.as_deref(), Some("DataRepository"));
    let helper_call = pc[0].1.iter().find(|c| c.method_name == "helperFn").unwrap();
    assert_eq!(helper_call.receiver_type, None);
    let fmt_call = pc[0].1.iter().find(|c| c.method_name == "format").unwrap();
    assert_eq!(fmt_call.receiver_type.as_deref(), Some("Formatter"));
}

#[test]
fn test_ts_no_calls_empty_body() {
    let source = r#"class EmptyService {
    doNothing(): void {}
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites) = parse_typescript_definitions(&mut parser, source, 0);

    let ni = defs.iter().position(|d| d.name == "doNothing").unwrap();
    let nc: Vec<_> = call_sites.iter().filter(|(i, _)| *i == ni).collect();
    assert!(nc.is_empty(), "Expected no call sites for empty method");
}

#[test]
fn test_ts_class_field_type() {
    let source = r#"class CachedService {
    private cache: CacheService;
    lookup(): void {
        this.cache.get();
    }
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites) = parse_typescript_definitions(&mut parser, source, 0);

    let li = defs.iter().position(|d| d.name == "lookup").unwrap();
    let lc: Vec<_> = call_sites.iter().filter(|(i, _)| *i == li).collect();
    assert!(!lc.is_empty(), "Expected call sites for 'lookup'");
    let gc = lc[0].1.iter().find(|c| c.method_name == "get");
    assert!(gc.is_some(), "Expected call to 'get'");
    assert_eq!(gc.unwrap().receiver_type.as_deref(), Some("CacheService"));
}

#[test]
fn test_ts_csharp_callers_still_work() {
    let source = r#"
public class NotificationService {
    private readonly IEmailSender _sender;
    public NotificationService(IEmailSender sender) { _sender = sender; }
    public void Notify(string message) { _sender.Send(message); this.LogResult(); }
    private void LogResult() {}
}
"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_c_sharp::LANGUAGE.into()).unwrap();
    let (defs, cs) = parse_csharp_definitions(&mut parser, source, 0);

    let ni = defs.iter().position(|d| d.name == "Notify").unwrap();
    let nc: Vec<_> = cs.iter().filter(|(i, _)| *i == ni).collect();
    assert!(!nc.is_empty(), "Expected call sites for 'Notify' (C# regression)");

    let send = nc[0].1.iter().find(|c| c.method_name == "Send");
    assert!(send.is_some(), "Expected call to 'Send'");
    assert_eq!(send.unwrap().receiver_type.as_deref(), Some("IEmailSender"));

    let log = nc[0].1.iter().find(|c| c.method_name == "LogResult");
    assert!(log.is_some(), "Expected call to 'LogResult'");
    assert_eq!(log.unwrap().receiver_type.as_deref(), Some("NotificationService"));
}


#[test]
fn test_ts_inject_field_initializer() {
    let source = r#"class MyComponent {
    private readonly zone = inject(NgZone);
    private readonly userService = inject(UserService);
    run(): void {
        this.zone.run();
        this.userService.getUser();
    }
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites) = parse_typescript_definitions(&mut parser, source, 0);

    let ri = defs.iter().position(|d| d.name == "run").unwrap();
    let rc: Vec<_> = call_sites.iter().filter(|(i, _)| *i == ri).collect();
    assert!(!rc.is_empty(), "Expected call sites for 'run' method");

    let zone_call = rc[0].1.iter().find(|c| c.method_name == "run" && c.receiver_type.is_some());
    assert!(zone_call.is_some(), "Expected call to 'zone.run()'");
    assert_eq!(zone_call.unwrap().receiver_type.as_deref(), Some("NgZone"));

    let user_call = rc[0].1.iter().find(|c| c.method_name == "getUser");
    assert!(user_call.is_some(), "Expected call to 'userService.getUser()'");
    assert_eq!(user_call.unwrap().receiver_type.as_deref(), Some("UserService"));
}

#[test]
fn test_ts_inject_constructor_assignment() {
    let source = r#"class MyComponent {
    constructor() {
        this.store = inject(Store);
        this.router = inject(Router);
    }
    navigate(): void {
        this.store.dispatch();
        this.router.navigate();
    }
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites) = parse_typescript_definitions(&mut parser, source, 0);

    let ni = defs.iter().position(|d| d.name == "navigate").unwrap();
    let nc: Vec<_> = call_sites.iter().filter(|(i, _)| *i == ni).collect();
    assert!(!nc.is_empty(), "Expected call sites for 'navigate' method");

    let store_call = nc[0].1.iter().find(|c| c.method_name == "dispatch");
    assert!(store_call.is_some(), "Expected call to 'store.dispatch()'");
    assert_eq!(store_call.unwrap().receiver_type.as_deref(), Some("Store"));

    let router_call = nc[0].1.iter().find(|c| c.method_name == "navigate" && c.receiver_type.is_some());
    assert!(router_call.is_some(), "Expected call to 'router.navigate()'");
    assert_eq!(router_call.unwrap().receiver_type.as_deref(), Some("Router"));
}

#[test]
fn test_ts_inject_with_generic() {
    let source = r#"class MyComponent {
    private store = inject(Store<AppState>);
    doWork(): void {
        this.store.dispatch();
    }
}"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites) = parse_typescript_definitions(&mut parser, source, 0);

    let di = defs.iter().position(|d| d.name == "doWork").unwrap();
    let dc: Vec<_> = call_sites.iter().filter(|(i, _)| *i == di).collect();
    assert!(!dc.is_empty(), "Expected call sites for 'doWork' method");

    let store_call = dc[0].1.iter().find(|c| c.method_name == "dispatch");
    assert!(store_call.is_some(), "Expected call to 'store.dispatch()'");
    assert_eq!(store_call.unwrap().receiver_type.as_deref(), Some("Store"));
}


// ─── TypeScript Interface Resolution Tests ───────────────────────────

#[test]
fn test_ts_interface_implements_extracted() {
    let source = r#"
interface IUserService {
    getUser(): void;
}

class UserService implements IUserService {
    getUser(): void {}
}
"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, _call_sites) = parse_typescript_definitions(&mut parser, source, 0);

    let class_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Class).collect();
    assert_eq!(class_defs.len(), 1);
    assert_eq!(class_defs[0].name, "UserService");
    assert!(
        class_defs[0].base_types.iter().any(|b| b.contains("IUserService")),
        "Expected base_types to contain 'IUserService', got: {:?}",
        class_defs[0].base_types
    );
}

#[test]
fn test_ts_interface_call_through_field() {
    let source = r#"
interface IOrderService {
    processOrder(): void;
}

class OrderProcessor {
    constructor(private orderService: IOrderService) {}
    run(): void {
        this.orderService.processOrder();
    }
}
"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, call_sites) = parse_typescript_definitions(&mut parser, source, 0);

    let ri = defs.iter().position(|d| d.name == "run").unwrap();
    let rc: Vec<_> = call_sites.iter().filter(|(i, _)| *i == ri).collect();
    assert!(!rc.is_empty(), "Expected call sites for 'run' method");

    let po = rc[0].1.iter().find(|c| c.method_name == "processOrder");
    assert!(po.is_some(), "Expected call to 'processOrder'");
    assert_eq!(
        po.unwrap().receiver_type.as_deref(),
        Some("IOrderService"),
        "Expected receiver_type to be 'IOrderService'"
    );
}

#[test]
fn test_ts_multiple_implements() {
    let source = r#"
interface IReader {
    read(): void;
}
interface IWriter {
    write(): void;
}
class DataService implements IReader, IWriter {
    read(): void {}
    write(): void {}
}
"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, _call_sites) = parse_typescript_definitions(&mut parser, source, 0);

    let class_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Class && d.name == "DataService").collect();
    assert_eq!(class_defs.len(), 1);
    assert!(
        class_defs[0].base_types.iter().any(|b| b.contains("IReader")),
        "Expected base_types to contain 'IReader', got: {:?}",
        class_defs[0].base_types
    );
    assert!(
        class_defs[0].base_types.iter().any(|b| b.contains("IWriter")),
        "Expected base_types to contain 'IWriter', got: {:?}",
        class_defs[0].base_types
    );
}

#[test]
fn test_ts_extends_and_implements() {
    let source = r#"
class BaseService {
    init(): void {}
}
interface IAdminService {
    manage(): void;
}
class AdminService extends BaseService implements IAdminService {
    manage(): void {}
}
"#;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
    let (defs, _call_sites) = parse_typescript_definitions(&mut parser, source, 0);

    let class_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Class && d.name == "AdminService").collect();
    assert_eq!(class_defs.len(), 1);
    assert!(
        class_defs[0].base_types.iter().any(|b| b.contains("BaseService")),
        "Expected base_types to contain 'BaseService', got: {:?}",
        class_defs[0].base_types
    );
    assert!(
        class_defs[0].base_types.iter().any(|b| b.contains("IAdminService")),
        "Expected base_types to contain 'IAdminService', got: {:?}",
        class_defs[0].base_types
    );
}


// ─── Non-UTF8 / Lossy Parsing Tests ──────────────────────────────────

#[test]
fn test_parse_csharp_with_non_utf8_byte_in_comment() {
    // Simulate a file with a Windows-1252 right single quote (0x92) in a comment.
    // After from_utf8_lossy, the byte becomes the replacement character U+FFFD.
    // The parser should still extract all definitions successfully.
    let raw_bytes: Vec<u8> = b"using System;

namespace TestApp
{
    /// <summary>
    /// Service for processing data. It\x92s important to handle edge cases.
    /// </summary>
    public class DataProcessor : BaseService
    {
        private readonly string _name;

        public DataProcessor(string name)
        {
            _name = name;
        }

        public void Process(int count)
        {
            // do work
        }
    }
}
".to_vec();

    // Verify the raw bytes are NOT valid UTF-8
    assert!(String::from_utf8(raw_bytes.clone()).is_err(),
        "Raw bytes should not be valid UTF-8 due to 0x92 byte");

    // Apply lossy conversion (same as our fix)
    let source = String::from_utf8_lossy(&raw_bytes).into_owned();
    assert!(source.contains('\u{FFFD}'), "Lossy conversion should insert replacement character");

    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_c_sharp::LANGUAGE.into()).unwrap();
    let (defs, _calls) = parse_csharp_definitions(&mut parser, &source, 0);

    // Should find: class DataProcessor, constructor, method Process, field _name
    let class_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Class).collect();
    assert_eq!(class_defs.len(), 1, "Should find DataProcessor class");
    assert_eq!(class_defs[0].name, "DataProcessor");
    assert!(class_defs[0].base_types.contains(&"BaseService".to_string()));

    let method_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Method).collect();
    assert_eq!(method_defs.len(), 1, "Should find Process method");
    assert_eq!(method_defs[0].name, "Process");

    let ctor_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Constructor).collect();
    assert_eq!(ctor_defs.len(), 1, "Should find constructor");
}

#[test]
fn test_read_file_lossy_with_valid_utf8() {
    let dir = std::env::temp_dir().join("search_test_lossy_valid");
    let _ = std::fs::create_dir_all(&dir);
    let file_path = dir.join("valid.cs");
    std::fs::write(&file_path, "public class ValidService {}").unwrap();

    let (content, was_lossy) = search::read_file_lossy(&file_path).unwrap();
    assert!(!was_lossy, "Valid UTF-8 file should not be lossy");
    assert!(content.contains("ValidService"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_read_file_lossy_with_non_utf8_byte() {
    let dir = std::env::temp_dir().join("search_test_lossy_invalid");
    let _ = std::fs::create_dir_all(&dir);
    let file_path = dir.join("invalid.cs");

    // Write a file with 0x92 byte (Windows-1252 right single quote)
    let mut content = b"// Comment: you\x92re a dev\n".to_vec();
    content.extend_from_slice(b"public class TestService {}\n");
    std::fs::write(&file_path, &content).unwrap();

    let (result, was_lossy) = search::read_file_lossy(&file_path).unwrap();
    assert!(was_lossy, "Non-UTF8 file should be lossy");
    assert!(result.contains("TestService"), "Should still read the file content");
    assert!(result.contains('\u{FFFD}'), "Should contain replacement character");

    let _ = std::fs::remove_dir_all(&dir);
}
