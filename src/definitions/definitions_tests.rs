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
        path_to_id: HashMap::new(), method_calls: HashMap::new(),
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
        path_to_id: HashMap::new(), method_calls: HashMap::new(),
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
        method_calls: HashMap::new(),
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
        method_calls: HashMap::new(),
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
        path_to_id: HashMap::new(), method_calls: HashMap::new(),
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
        path_to_id: HashMap::new(), method_calls: HashMap::new(),
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
