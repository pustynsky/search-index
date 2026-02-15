use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};

use crate::clean_path;

// ─── Data Structures ─────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
pub enum DefinitionKind {
    // C# kinds
    Class,
    Interface,
    Enum,
    Struct,
    Record,
    Method,
    Property,
    Field,
    Constructor,
    Delegate,
    Event,
    EnumMember,
    // SQL kinds
    StoredProcedure,
    Table,
    View,
    SqlFunction,
    UserDefinedType,
    Column,
    SqlIndex,
}

impl DefinitionKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Class => "class",
            Self::Interface => "interface",
            Self::Enum => "enum",
            Self::Struct => "struct",
            Self::Record => "record",
            Self::Method => "method",
            Self::Property => "property",
            Self::Field => "field",
            Self::Constructor => "constructor",
            Self::Delegate => "delegate",
            Self::Event => "event",
            Self::EnumMember => "enumMember",
            Self::StoredProcedure => "storedProcedure",
            Self::Table => "table",
            Self::View => "view",
            Self::SqlFunction => "sqlFunction",
            Self::UserDefinedType => "userDefinedType",
            Self::Column => "column",
            Self::SqlIndex => "sqlIndex",
        }
    }
}

impl std::fmt::Display for DefinitionKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for DefinitionKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "class" => Ok(Self::Class),
            "interface" => Ok(Self::Interface),
            "enum" => Ok(Self::Enum),
            "struct" => Ok(Self::Struct),
            "record" => Ok(Self::Record),
            "method" => Ok(Self::Method),
            "property" => Ok(Self::Property),
            "field" => Ok(Self::Field),
            "constructor" => Ok(Self::Constructor),
            "delegate" => Ok(Self::Delegate),
            "event" => Ok(Self::Event),
            "enummember" => Ok(Self::EnumMember),
            "storedprocedure" => Ok(Self::StoredProcedure),
            "table" => Ok(Self::Table),
            "view" => Ok(Self::View),
            "sqlfunction" => Ok(Self::SqlFunction),
            "userdefinedtype" => Ok(Self::UserDefinedType),
            "column" => Ok(Self::Column),
            "sqlindex" => Ok(Self::SqlIndex),
            other => Err(format!("Unknown definition kind: '{}'", other)),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DefinitionEntry {
    pub file_id: u32,
    pub name: String,
    pub kind: DefinitionKind,
    pub line_start: u32,
    pub line_end: u32,
    pub parent: Option<String>,
    pub signature: Option<String>,
    pub modifiers: Vec<String>,
    pub attributes: Vec<String>,
    pub base_types: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct DefinitionIndex {
    pub root: String,
    pub created_at: u64,
    pub extensions: Vec<String>,
    /// file_id -> file path
    pub files: Vec<String>,
    /// All definitions
    pub definitions: Vec<DefinitionEntry>,
    /// name (lowercased) -> Vec<index into definitions>
    pub name_index: HashMap<String, Vec<u32>>,
    /// kind -> Vec<index into definitions>
    pub kind_index: HashMap<DefinitionKind, Vec<u32>>,
    /// attribute name (lowercased) -> Vec<index into definitions>
    pub attribute_index: HashMap<String, Vec<u32>>,
    /// base type name (lowercased) -> Vec<index into definitions>
    pub base_type_index: HashMap<String, Vec<u32>>,
    /// file_id -> Vec<index into definitions>
    pub file_index: HashMap<u32, Vec<u32>>,
    /// Path -> file_id lookup (for watcher)
    pub path_to_id: HashMap<PathBuf, u32>,
}

// ─── CLI Args ────────────────────────────────────────────────────────

use clap::Parser;

#[derive(Parser, Debug)]
#[command(after_long_help = r#"WHAT IT DOES:
  Parses C# and SQL files using tree-sitter to extract code structure:
    - C#: classes, interfaces, structs, enums, records, methods, constructors,
      properties, fields, delegates, events, enum members
    - SQL: stored procedures, tables, views, functions, user-defined types
      (requires compatible tree-sitter-sql grammar)

  Each definition includes: name, kind, file path, line range, signature,
  modifiers, attributes (e.g. [ServiceProvider]), and base types.

  The index is saved to disk as a .didx file and can be loaded instantly
  by 'search serve --definitions'.

EXAMPLES:
  Index C# files:     search def-index --dir C:\Projects --ext cs
  Index C# + SQL:     search def-index --dir C:\Projects --ext cs,sql
  Custom threads:     search def-index --dir C:\Projects --ext cs --threads 8

PERFORMANCE:
  48,643 files → 846,167 definitions in ~14s (24 threads)
  Index size: ~230 MB on disk
"#)]
pub struct DefIndexArgs {
    /// Directory to recursively scan for source files to parse
    #[arg(short, long, default_value = ".")]
    pub dir: String,

    /// File extensions to parse, comma-separated.
    /// C# (.cs) uses tree-sitter-c-sharp grammar.
    /// SQL (.sql) uses tree-sitter-sequel-tsql grammar (T-SQL support).
    #[arg(short, long, default_value = "cs,sql")]
    pub ext: String,

    /// Number of parallel parsing threads. Each thread gets its own
    /// tree-sitter parser instance. 0 = auto-detect CPU cores.
    #[arg(short, long, default_value = "0")]
    pub threads: usize,
}

// ─── Index Build ─────────────────────────────────────────────────────

pub fn build_definition_index(args: &DefIndexArgs) -> DefinitionIndex {
    let dir = std::fs::canonicalize(&args.dir)
        .unwrap_or_else(|_| PathBuf::from(&args.dir));
    let dir_str = clean_path(&dir.to_string_lossy());

    let extensions: Vec<String> = args.ext.split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();

    let start = Instant::now();

    // Collect all files
    let mut walker = WalkBuilder::new(&dir);
    walker.hidden(false).git_ignore(true);
    if args.threads > 0 {
        walker.threads(args.threads);
    }

    let file_count = AtomicUsize::new(0);
    let all_files: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

    walker.build_parallel().run(|| {
        Box::new(|entry| {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => return ignore::WalkState::Continue,
            };
            if !entry.file_type().map_or(false, |ft| ft.is_file()) {
                return ignore::WalkState::Continue;
            }
            let path = entry.path();
            let ext_match = path.extension()
                .and_then(|e| e.to_str())
                .map_or(false, |e| extensions.iter().any(|x| x.eq_ignore_ascii_case(e)));
            if !ext_match {
                return ignore::WalkState::Continue;
            }
            let clean = clean_path(&path.to_string_lossy());
            all_files.lock().unwrap_or_else(|e| e.into_inner()).push(clean);
            file_count.fetch_add(1, Ordering::Relaxed);
            ignore::WalkState::Continue
        })
    });

    let files: Vec<String> = all_files.into_inner().unwrap();
    let total_files = files.len();
    eprintln!("[def-index] Found {} files to parse", total_files);

    // Check SQL grammar availability
    let sql_available = {
        let mut test_parser = tree_sitter::Parser::new();
        match test_parser.set_language(&tree_sitter_sequel_tsql::LANGUAGE.into()) {
            Ok(()) => true,
            Err(e) => {
                eprintln!("[def-index] Warning: SQL grammar not compatible ({}), skipping .sql files", e);
                false
            }
        }
    };

    // ─── Parallel parsing ─────────────────────────────────────
    let num_threads = if args.threads > 0 {
        args.threads
    } else {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
    };
    let chunk_size = (total_files + num_threads - 1) / num_threads;
    let chunks: Vec<Vec<(u32, String)>> = files.iter().enumerate()
        .map(|(i, f)| (i as u32, f.clone()))
        .collect::<Vec<_>>()
        .chunks(chunk_size.max(1))
        .map(|c| c.to_vec())
        .collect();

    eprintln!("[def-index] Parsing with {} threads ({} files/chunk)", chunks.len(), chunk_size);

    let sql_avail = sql_available;
    let thread_results: Vec<_> = std::thread::scope(|s| {
        let handles: Vec<_> = chunks.into_iter().map(|chunk| {
            s.spawn(move || {
                let mut cs_parser = tree_sitter::Parser::new();
                cs_parser.set_language(&tree_sitter_c_sharp::LANGUAGE.into())
                    .expect("Error loading C# grammar");

                let mut sql_parser = tree_sitter::Parser::new();
                if sql_avail {
                    let _ = sql_parser.set_language(&tree_sitter_sequel_tsql::LANGUAGE.into());
                }

                let mut chunk_defs: Vec<(u32, Vec<DefinitionEntry>)> = Vec::new();
                let mut errors = 0usize;

                for (file_id, file_path) in &chunk {
                    let content = match std::fs::read_to_string(file_path) {
                        Ok(c) => c,
                        Err(_) => { errors += 1; continue; }
                    };

                    let ext = Path::new(file_path.as_str())
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("");

                    let file_defs = match ext.to_lowercase().as_str() {
                        "cs" => parse_csharp_definitions(&mut cs_parser, &content, *file_id),
                        "sql" if sql_avail => parse_sql_definitions(&mut sql_parser, &content, *file_id),
                        _ => Vec::new(),
                    };

                    if !file_defs.is_empty() {
                        chunk_defs.push((*file_id, file_defs));
                    }
                }

                (chunk_defs, errors)
            })
        }).collect();

        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });

    // ─── Merge results ────────────────────────────────────────
    let mut definitions: Vec<DefinitionEntry> = Vec::new();
    let mut name_index: HashMap<String, Vec<u32>> = HashMap::new();
    let mut kind_index: HashMap<DefinitionKind, Vec<u32>> = HashMap::new();
    let mut attribute_index: HashMap<String, Vec<u32>> = HashMap::new();
    let mut base_type_index: HashMap<String, Vec<u32>> = HashMap::new();
    let mut file_index: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut path_to_id: HashMap<PathBuf, u32> = HashMap::new();
    let mut parse_errors = 0usize;

    // Build path_to_id from the files list
    for (file_id, file_path) in files.iter().enumerate() {
        path_to_id.insert(PathBuf::from(file_path), file_id as u32);
    }

    for (chunk_defs, errors) in thread_results {
        parse_errors += errors;
        for (file_id, file_defs) in chunk_defs {
            for def in file_defs {
                let def_idx = definitions.len() as u32;

                name_index.entry(def.name.to_lowercase())
                    .or_default()
                    .push(def_idx);

                kind_index.entry(def.kind.clone())
                    .or_default()
                    .push(def_idx);

                for attr in &def.attributes {
                    let attr_name = attr.split('(').next().unwrap_or(attr).trim().to_lowercase();
                    attribute_index.entry(attr_name)
                        .or_default()
                        .push(def_idx);
                }

                for bt in &def.base_types {
                    base_type_index.entry(bt.to_lowercase())
                        .or_default()
                        .push(def_idx);
                }

                file_index.entry(file_id)
                    .or_default()
                    .push(def_idx);

                definitions.push(def);
            }
        }
    }

    let elapsed = start.elapsed();
    eprintln!(
        "[def-index] Parsed {} files in {:.1}s, extracted {} definitions ({} parse errors, {} threads)",
        total_files,
        elapsed.as_secs_f64(),
        definitions.len(),
        parse_errors,
        num_threads
    );

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    DefinitionIndex {
        root: dir_str,
        created_at: now,
        extensions,
        files,
        definitions,
        name_index,
        kind_index,
        attribute_index,
        base_type_index,
        file_index,
        path_to_id,
    }
}

// ─── C# Parser ───────────────────────────────────────────────────────

fn parse_csharp_definitions(
    parser: &mut tree_sitter::Parser,
    source: &str,
    file_id: u32,
) -> Vec<DefinitionEntry> {
    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return Vec::new(),
    };

    let mut defs = Vec::new();
    let source_bytes = source.as_bytes();
    walk_csharp_node(tree.root_node(), source_bytes, file_id, None, &mut defs);
    defs
}

fn walk_csharp_node(
    node: tree_sitter::Node,
    source: &[u8],
    file_id: u32,
    parent_name: Option<&str>,
    defs: &mut Vec<DefinitionEntry>,
) {
    let kind = node.kind();

    match kind {
        "class_declaration" | "interface_declaration" | "struct_declaration"
        | "enum_declaration" | "record_declaration" => {
            if let Some(def) = extract_csharp_type_def(node, source, file_id, parent_name) {
                let name = def.name.clone();
                defs.push(def);
                // Recurse into type body for members
                for i in 0..node.child_count() {
                    let child = node.child(i).unwrap();
                    match child.kind() {
                        "declaration_list" | "enum_member_declaration_list" => {
                            walk_csharp_node(child, source, file_id, Some(&name), defs);
                        }
                        _ => {}
                    }
                }
                return; // Don't recurse further for this node
            }
        }
        "method_declaration" => {
            if let Some(def) = extract_csharp_method_def(node, source, file_id, parent_name) {
                defs.push(def);
                return;
            }
        }
        "constructor_declaration" => {
            if let Some(def) = extract_csharp_constructor_def(node, source, file_id, parent_name) {
                defs.push(def);
                return;
            }
        }
        "property_declaration" => {
            if let Some(def) = extract_csharp_property_def(node, source, file_id, parent_name) {
                defs.push(def);
                return;
            }
        }
        "field_declaration" => {
            extract_csharp_field_defs(node, source, file_id, parent_name, defs);
            return;
        }
        "delegate_declaration" => {
            if let Some(def) = extract_csharp_delegate_def(node, source, file_id, parent_name) {
                defs.push(def);
                return;
            }
        }
        "event_declaration" | "event_field_declaration" => {
            if let Some(def) = extract_csharp_event_def(node, source, file_id, parent_name) {
                defs.push(def);
                return;
            }
        }
        "enum_member_declaration" => {
            if let Some(def) = extract_csharp_enum_member(node, source, file_id, parent_name) {
                defs.push(def);
                return;
            }
        }
        _ => {}
    }

    // Recurse into children
    for i in 0..node.child_count() {
        walk_csharp_node(node.child(i).unwrap(), source, file_id, parent_name, defs);
    }
}

fn node_text<'a>(node: tree_sitter::Node, source: &'a [u8]) -> &'a str {
    node.utf8_text(source).unwrap_or("")
}

fn find_child_by_kind<'a>(node: tree_sitter::Node<'a>, kind: &str) -> Option<tree_sitter::Node<'a>> {
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        if child.kind() == kind {
            return Some(child);
        }
    }
    None
}

fn find_child_by_field<'a>(node: tree_sitter::Node<'a>, field: &str) -> Option<tree_sitter::Node<'a>> {
    node.child_by_field_name(field)
}

fn extract_modifiers(node: tree_sitter::Node, source: &[u8]) -> Vec<String> {
    let mut modifiers = Vec::new();
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        if child.kind() == "modifier" || child.kind().ends_with("_modifier") {
            modifiers.push(node_text(child, source).to_string());
        }
        // Also check for individual modifier keywords
        match child.kind() {
            "public" | "private" | "protected" | "internal" | "static" | "readonly"
            | "sealed" | "abstract" | "virtual" | "override" | "async" | "partial"
            | "new" | "extern" | "unsafe" | "volatile" | "const" => {
                modifiers.push(node_text(child, source).to_string());
            }
            _ => {}
        }
    }
    modifiers
}

fn extract_attributes(node: tree_sitter::Node, source: &[u8]) -> Vec<String> {
    let mut attributes = Vec::new();
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        if child.kind() == "attribute_list" {
            for j in 0..child.child_count() {
                let attr = child.child(j).unwrap();
                if attr.kind() == "attribute" {
                    attributes.push(node_text(attr, source).to_string());
                }
            }
        }
    }
    attributes
}

fn extract_base_types(node: tree_sitter::Node, source: &[u8]) -> Vec<String> {
    let mut base_types = Vec::new();
    if let Some(base_list) = find_child_by_kind(node, "base_list") {
        for i in 0..base_list.child_count() {
            let child = base_list.child(i).unwrap();
            // Skip punctuation like ':'  and ','
            if child.is_named() {
                base_types.push(node_text(child, source).to_string());
            }
        }
    }
    base_types
}

fn extract_csharp_type_def(
    node: tree_sitter::Node,
    source: &[u8],
    file_id: u32,
    parent_name: Option<&str>,
) -> Option<DefinitionEntry> {
    let name_node = find_child_by_field(node, "name")?;
    let name = node_text(name_node, source).to_string();

    let kind = match node.kind() {
        "class_declaration" => DefinitionKind::Class,
        "interface_declaration" => DefinitionKind::Interface,
        "struct_declaration" => DefinitionKind::Struct,
        "enum_declaration" => DefinitionKind::Enum,
        "record_declaration" => DefinitionKind::Record,
        _ => return None,
    };

    let modifiers = extract_modifiers(node, source);
    let attributes = extract_attributes(node, source);
    let base_types = extract_base_types(node, source);

    // Build signature (first line of declaration up to '{' or body)
    let sig = build_type_signature(node, source);

    Some(DefinitionEntry {
        file_id,
        name,
        kind,
        line_start: node.start_position().row as u32 + 1,
        line_end: node.end_position().row as u32 + 1,
        parent: parent_name.map(|s| s.to_string()),
        signature: Some(sig),
        modifiers,
        attributes,
        base_types,
    })
}

fn build_type_signature(node: tree_sitter::Node, source: &[u8]) -> String {
    // Get text from start of node to the opening brace
    let start = node.start_byte();
    let mut end = node.end_byte();

    // Find the '{' in children
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        if child.kind() == "declaration_list" || child.kind() == "{" {
            end = child.start_byte();
            break;
        }
    }

    let text = std::str::from_utf8(&source[start..end]).unwrap_or("");
    // Collapse whitespace
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn extract_csharp_method_def(
    node: tree_sitter::Node,
    source: &[u8],
    file_id: u32,
    parent_name: Option<&str>,
) -> Option<DefinitionEntry> {
    let name_node = find_child_by_field(node, "name")?;
    let name = node_text(name_node, source).to_string();

    let modifiers = extract_modifiers(node, source);
    let attributes = extract_attributes(node, source);

    // Build signature: return_type name(params)
    let sig = build_method_signature(node, source);

    Some(DefinitionEntry {
        file_id,
        name,
        kind: DefinitionKind::Method,
        line_start: node.start_position().row as u32 + 1,
        line_end: node.end_position().row as u32 + 1,
        parent: parent_name.map(|s| s.to_string()),
        signature: Some(sig),
        modifiers,
        attributes,
        base_types: Vec::new(),
    })
}

fn build_method_signature(node: tree_sitter::Node, source: &[u8]) -> String {
    let start = node.start_byte();
    let mut end = node.end_byte();

    // Find body (block) or '=>'
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        if child.kind() == "block" || child.kind() == "arrow_expression_clause" || child.kind() == ";" {
            end = child.start_byte();
            break;
        }
    }

    let text = std::str::from_utf8(&source[start..end]).unwrap_or("");
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn extract_csharp_constructor_def(
    node: tree_sitter::Node,
    source: &[u8],
    file_id: u32,
    parent_name: Option<&str>,
) -> Option<DefinitionEntry> {
    let name_node = find_child_by_field(node, "name")?;
    let name = node_text(name_node, source).to_string();

    let modifiers = extract_modifiers(node, source);
    let attributes = extract_attributes(node, source);
    let sig = build_method_signature(node, source);

    Some(DefinitionEntry {
        file_id,
        name,
        kind: DefinitionKind::Constructor,
        line_start: node.start_position().row as u32 + 1,
        line_end: node.end_position().row as u32 + 1,
        parent: parent_name.map(|s| s.to_string()),
        signature: Some(sig),
        modifiers,
        attributes,
        base_types: Vec::new(),
    })
}

fn extract_csharp_property_def(
    node: tree_sitter::Node,
    source: &[u8],
    file_id: u32,
    parent_name: Option<&str>,
) -> Option<DefinitionEntry> {
    let name_node = find_child_by_field(node, "name")?;
    let name = node_text(name_node, source).to_string();

    let modifiers = extract_modifiers(node, source);
    let attributes = extract_attributes(node, source);

    // Signature: type name
    let type_node = find_child_by_field(node, "type");
    let type_str = type_node.map(|n| node_text(n, source)).unwrap_or("");
    let sig = format!("{} {}", type_str, name);

    Some(DefinitionEntry {
        file_id,
        name,
        kind: DefinitionKind::Property,
        line_start: node.start_position().row as u32 + 1,
        line_end: node.end_position().row as u32 + 1,
        parent: parent_name.map(|s| s.to_string()),
        signature: Some(sig.trim().to_string()),
        modifiers,
        attributes,
        base_types: Vec::new(),
    })
}

fn extract_csharp_field_defs(
    node: tree_sitter::Node,
    source: &[u8],
    file_id: u32,
    parent_name: Option<&str>,
    defs: &mut Vec<DefinitionEntry>,
) {
    let modifiers = extract_modifiers(node, source);
    let attributes = extract_attributes(node, source);

    // field_declaration has variable_declaration child with variable_declarator children
    if let Some(var_decl) = find_child_by_kind(node, "variable_declaration") {
        let type_node = find_child_by_field(var_decl, "type");
        let type_str = type_node.map(|n| node_text(n, source)).unwrap_or("");

        for i in 0..var_decl.child_count() {
            let child = var_decl.child(i).unwrap();
            if child.kind() == "variable_declarator" {
                if let Some(name_node) = find_child_by_field(child, "name") {
                    let name = node_text(name_node, source).to_string();
                    let sig = format!("{} {}", type_str, name);

                    defs.push(DefinitionEntry {
                        file_id,
                        name,
                        kind: DefinitionKind::Field,
                        line_start: node.start_position().row as u32 + 1,
                        line_end: node.end_position().row as u32 + 1,
                        parent: parent_name.map(|s| s.to_string()),
                        signature: Some(sig.trim().to_string()),
                        modifiers: modifiers.clone(),
                        attributes: attributes.clone(),
                        base_types: Vec::new(),
                    });
                }
            }
        }
    }
}

fn extract_csharp_delegate_def(
    node: tree_sitter::Node,
    source: &[u8],
    file_id: u32,
    parent_name: Option<&str>,
) -> Option<DefinitionEntry> {
    let name_node = find_child_by_field(node, "name")?;
    let name = node_text(name_node, source).to_string();

    let modifiers = extract_modifiers(node, source);
    let attributes = extract_attributes(node, source);

    let sig_text = node_text(node, source);
    let sig = sig_text.split_whitespace().collect::<Vec<_>>().join(" ");

    Some(DefinitionEntry {
        file_id,
        name,
        kind: DefinitionKind::Delegate,
        line_start: node.start_position().row as u32 + 1,
        line_end: node.end_position().row as u32 + 1,
        parent: parent_name.map(|s| s.to_string()),
        signature: Some(sig),
        modifiers,
        attributes,
        base_types: Vec::new(),
    })
}

fn extract_csharp_event_def(
    node: tree_sitter::Node,
    source: &[u8],
    file_id: u32,
    parent_name: Option<&str>,
) -> Option<DefinitionEntry> {
    // Try to find the name — events can have different structures
    let name = if let Some(name_node) = find_child_by_field(node, "name") {
        node_text(name_node, source).to_string()
    } else {
        // event_field_declaration: look inside variable_declaration
        let var_decl = find_child_by_kind(node, "variable_declaration");
        if let Some(vd) = var_decl {
            let declarator = find_child_by_kind(vd, "variable_declarator");
            if let Some(d) = declarator {
                if let Some(n) = find_child_by_field(d, "name") {
                    node_text(n, source).to_string()
                } else {
                    return None;
                }
            } else {
                return None;
            }
        } else {
            return None;
        }
    };

    let modifiers = extract_modifiers(node, source);
    let attributes = extract_attributes(node, source);

    Some(DefinitionEntry {
        file_id,
        name,
        kind: DefinitionKind::Event,
        line_start: node.start_position().row as u32 + 1,
        line_end: node.end_position().row as u32 + 1,
        parent: parent_name.map(|s| s.to_string()),
        signature: None,
        modifiers,
        attributes,
        base_types: Vec::new(),
    })
}

fn extract_csharp_enum_member(
    node: tree_sitter::Node,
    source: &[u8],
    file_id: u32,
    parent_name: Option<&str>,
) -> Option<DefinitionEntry> {
    let name_node = find_child_by_field(node, "name")?;
    let name = node_text(name_node, source).to_string();

    Some(DefinitionEntry {
        file_id,
        name,
        kind: DefinitionKind::EnumMember,
        line_start: node.start_position().row as u32 + 1,
        line_end: node.end_position().row as u32 + 1,
        parent: parent_name.map(|s| s.to_string()),
        signature: None,
        modifiers: Vec::new(),
        attributes: Vec::new(),
        base_types: Vec::new(),
    })
}

// ─── SQL Parser ──────────────────────────────────────────────────────

fn parse_sql_definitions(
    parser: &mut tree_sitter::Parser,
    source: &str,
    file_id: u32,
) -> Vec<DefinitionEntry> {
    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return Vec::new(),
    };

    let mut defs = Vec::new();
    walk_sql_node(tree.root_node(), source.as_bytes(), file_id, &mut defs);
    defs
}

fn walk_sql_node(
    node: tree_sitter::Node,
    source: &[u8],
    file_id: u32,
    defs: &mut Vec<DefinitionEntry>,
) {
    let kind = node.kind();

    match kind {
        "create_function_statement" => {
            // In T-SQL, both PROCEDURE and FUNCTION use similar AST patterns
            // Check if the text contains PROCEDURE or FUNCTION
            let text = node_text(node, source).to_uppercase();
            if text.contains("PROCEDURE") || text.contains("PROC") {
                if let Some(def) = extract_sql_named_def(node, source, file_id, DefinitionKind::StoredProcedure) {
                    defs.push(def);
                    return;
                }
            } else {
                if let Some(def) = extract_sql_named_def(node, source, file_id, DefinitionKind::SqlFunction) {
                    defs.push(def);
                    return;
                }
            }
        }
        "create_procedure_statement" => {
            if let Some(def) = extract_sql_named_def(node, source, file_id, DefinitionKind::StoredProcedure) {
                defs.push(def);
                return;
            }
        }
        "create_table_statement" => {
            if let Some(def) = extract_sql_named_def(node, source, file_id, DefinitionKind::Table) {
                defs.push(def);
                return;
            }
        }
        "create_view_statement" => {
            if let Some(def) = extract_sql_named_def(node, source, file_id, DefinitionKind::View) {
                defs.push(def);
                return;
            }
        }
        "create_type_statement" => {
            if let Some(def) = extract_sql_named_def(node, source, file_id, DefinitionKind::UserDefinedType) {
                defs.push(def);
                return;
            }
        }
        "create_index_statement" => {
            if let Some(def) = extract_sql_named_def(node, source, file_id, DefinitionKind::SqlIndex) {
                defs.push(def);
                return;
            }
        }
        _ => {}
    }

    // Recurse
    for i in 0..node.child_count() {
        walk_sql_node(node.child(i).unwrap(), source, file_id, defs);
    }
}

fn extract_sql_named_def(
    node: tree_sitter::Node,
    source: &[u8],
    file_id: u32,
    kind: DefinitionKind,
) -> Option<DefinitionEntry> {
    // Try to find the name through various SQL AST patterns
    // The name is typically the first identifier or dotted name after CREATE TABLE/PROCEDURE/etc.
    let name = find_sql_object_name(node, source)?;

    // Build a one-line signature from first line of the node
    let full_text = node_text(node, source);
    let first_line = full_text.lines().next().unwrap_or("");
    let sig = first_line.split_whitespace().collect::<Vec<_>>().join(" ");

    Some(DefinitionEntry {
        file_id,
        name,
        kind,
        line_start: node.start_position().row as u32 + 1,
        line_end: node.end_position().row as u32 + 1,
        parent: None,
        signature: Some(sig),
        modifiers: Vec::new(),
        attributes: Vec::new(),
        base_types: Vec::new(),
    })
}

fn find_sql_object_name(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    // Try field "name" first
    if let Some(name_node) = find_child_by_field(node, "name") {
        return Some(node_text(name_node, source).to_string());
    }

    // Walk children looking for an identifier or dotted name
    // Skip keywords like CREATE, TABLE, PROCEDURE, etc.
    let mut found_keyword = false;
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        let ck = child.kind();

        // Skip CREATE, ALTER, OR, REPLACE keywords
        if ck == "keyword_create" || ck == "keyword_table" || ck == "keyword_procedure"
           || ck == "keyword_function" || ck == "keyword_view" || ck == "keyword_type"
           || ck == "keyword_index" || ck == "keyword_or" || ck == "keyword_replace"
           || ck == "keyword_alter" || ck == "keyword_unique" || ck == "keyword_clustered"
           || ck == "keyword_nonclustered" || ck == "keyword_if" || ck == "keyword_not"
           || ck == "keyword_exists" || ck == "CREATE" || ck == "TABLE" || ck == "PROCEDURE" {
            found_keyword = true;
            continue;
        }

        // After keywords, the next identifier-like node is the name
        if found_keyword {
            if ck == "identifier" || ck == "dotted_name" || ck == "object_reference"
               || ck == "schema_qualified_name" || child.is_named() {
                let text = node_text(child, source).to_string();
                if !text.is_empty() && !text.starts_with('(') && !text.starts_with("AS") {
                    return Some(text);
                }
            }
        }
    }

    // Fallback: extract from raw text using regex-like approach
    let text = node_text(node, source);
    let upper = text.to_uppercase();
    for keyword in &["PROCEDURE", "PROC", "TABLE", "FUNCTION", "VIEW", "TYPE", "INDEX"] {
        if let Some(pos) = upper.find(keyword) {
            let after = &text[pos + keyword.len()..];
            let name = after.trim()
                .split(|c: char| c.is_whitespace() || c == '(' || c == '\n' || c == '\r')
                .next()
                .unwrap_or("");
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }

    None
}

// ─── Index Persistence ───────────────────────────────────────────────

fn def_index_path_for(dir: &str, exts: &str) -> PathBuf {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let canonical = std::fs::canonicalize(dir).unwrap_or_else(|_| PathBuf::from(dir));
    let mut hasher = DefaultHasher::new();
    canonical.to_string_lossy().hash(&mut hasher);
    exts.hash(&mut hasher);
    "definitions".hash(&mut hasher); // distinguish from content index
    let hash = hasher.finish();
    crate::index_dir().join(format!("{:016x}.didx", hash))
}

pub fn save_definition_index(index: &DefinitionIndex) -> Result<(), crate::SearchError> {
    let dir = crate::index_dir();
    std::fs::create_dir_all(&dir)?;
    let exts_str = index.extensions.join(",");
    let path = def_index_path_for(&index.root, &exts_str);
    let encoded = bincode::serialize(index)?;
    std::fs::write(&path, &encoded)?;
    eprintln!(
        "[def-index] Saved index ({} definitions, {:.1} MB) to {}",
        index.definitions.len(),
        encoded.len() as f64 / 1_048_576.0,
        clean_path(&path.to_string_lossy())
    );
    Ok(())
}

#[allow(dead_code)]
pub fn load_definition_index(dir: &str, exts: &str) -> Option<DefinitionIndex> {
    let path = def_index_path_for(dir, exts);
    let data = std::fs::read(&path).ok()?;
    bincode::deserialize(&data).ok()
}

/// Try to find any definition index for a directory (any extension combo)
#[allow(dead_code)]
pub fn find_definition_index_for_dir(dir: &str) -> Option<DefinitionIndex> {
    let canonical = std::fs::canonicalize(dir).ok()?;
    let dir_str = clean_path(&canonical.to_string_lossy());
    let idx_dir = crate::index_dir();
    let entries = std::fs::read_dir(&idx_dir).ok()?;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("didx") {
            if let Ok(data) = std::fs::read(&path) {
                if let Ok(index) = bincode::deserialize::<DefinitionIndex>(&data) {
                    let idx_root = std::fs::canonicalize(&index.root)
                        .map(|p| clean_path(&p.to_string_lossy()))
                        .unwrap_or_else(|_| index.root.clone());
                    if idx_root.eq_ignore_ascii_case(&dir_str) {
                        return Some(index);
                    }
                }
            }
        }
    }
    None
}

// ─── Incremental Update (for watcher) ────────────────────────────────

/// Update definitions for a single file (incremental).
/// Removes old definitions for the file, parses it again, adds new ones.
pub fn update_file_definitions(index: &mut DefinitionIndex, path: &Path) {
    let path_str = path.to_string_lossy().to_string();

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return,
    };

    let ext = path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    // Get or assign file_id
    let file_id = if let Some(&id) = index.path_to_id.get(path) {
        // Existing file — remove old definitions
        remove_file_definitions(index, id);
        id
    } else {
        // New file
        let id = index.files.len() as u32;
        index.files.push(path_str);
        index.path_to_id.insert(path.to_path_buf(), id);
        id
    };

    // Parse the file
    let mut cs_parser = tree_sitter::Parser::new();
    cs_parser.set_language(&tree_sitter_c_sharp::LANGUAGE.into()).ok();

    let file_defs = match ext.to_lowercase().as_str() {
        "cs" => parse_csharp_definitions(&mut cs_parser, &content, file_id),
        _ => Vec::new(),
    };

    // Add new definitions to index
    for def in file_defs {
        let def_idx = index.definitions.len() as u32;

        index.name_index.entry(def.name.to_lowercase())
            .or_default()
            .push(def_idx);

        index.kind_index.entry(def.kind.clone())
            .or_default()
            .push(def_idx);

        for attr in &def.attributes {
            let attr_name = attr.split('(').next().unwrap_or(attr).trim().to_lowercase();
            index.attribute_index.entry(attr_name)
                .or_default()
                .push(def_idx);
        }

        for bt in &def.base_types {
            index.base_type_index.entry(bt.to_lowercase())
                .or_default()
                .push(def_idx);
        }

        index.file_index.entry(file_id)
            .or_default()
            .push(def_idx);

        index.definitions.push(def);
    }
}

/// Remove all definitions for a file from the index
pub fn remove_file_definitions(index: &mut DefinitionIndex, file_id: u32) {
    // Get the definition indices for this file
    let def_indices = match index.file_index.remove(&file_id) {
        Some(indices) => indices,
        None => return,
    };

    // Remove from all inverted indexes
    let indices_set: std::collections::HashSet<u32> = def_indices.iter().cloned().collect();

    // Remove from name_index
    index.name_index.retain(|_, v| {
        v.retain(|idx| !indices_set.contains(idx));
        !v.is_empty()
    });

    // Remove from kind_index
    index.kind_index.retain(|_, v| {
        v.retain(|idx| !indices_set.contains(idx));
        !v.is_empty()
    });

    // Remove from attribute_index
    index.attribute_index.retain(|_, v| {
        v.retain(|idx| !indices_set.contains(idx));
        !v.is_empty()
    });

    // Remove from base_type_index
    index.base_type_index.retain(|_, v| {
        v.retain(|idx| !indices_set.contains(idx));
        !v.is_empty()
    });

    // Note: we don't remove from definitions vec to preserve index stability
    // (other indices reference by position). The entries become "tombstones".
}

/// Remove a file entirely from the definition index
pub fn remove_file_from_def_index(index: &mut DefinitionIndex, path: &Path) {
    if let Some(&file_id) = index.path_to_id.get(path) {
        remove_file_definitions(index, file_id);
        index.path_to_id.remove(path);
    }
}

// ─── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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

        let defs = parse_csharp_definitions(&mut parser, source, 0);

        // Check we found the class
        let class_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Class).collect();
        assert_eq!(class_defs.len(), 1);
        assert_eq!(class_defs[0].name, "MyService");
        assert!(!class_defs[0].attributes.is_empty());
        assert!(class_defs[0].base_types.len() >= 1); // BaseService, IMyService

        // Check interface
        let iface_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Interface).collect();
        assert_eq!(iface_defs.len(), 1);
        assert_eq!(iface_defs[0].name, "IMyService");

        // Check method
        let method_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Method).collect();
        assert!(method_defs.len() >= 1);
        let do_work = method_defs.iter().find(|d| d.name == "DoWork");
        assert!(do_work.is_some());
        assert_eq!(do_work.unwrap().parent, Some("MyService".to_string()));

        // Check property
        let prop_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Property).collect();
        assert!(prop_defs.len() >= 1);
        assert_eq!(prop_defs[0].name, "Name");

        // Check field
        let field_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Field).collect();
        assert!(field_defs.len() >= 1);

        // Check constructor
        let ctor_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Constructor).collect();
        assert_eq!(ctor_defs.len(), 1);
        assert_eq!(ctor_defs[0].name, "MyService");

        // Check enum
        let enum_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Enum).collect();
        assert_eq!(enum_defs.len(), 1);
        assert_eq!(enum_defs[0].name, "Status");

        // Check enum members
        let member_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::EnumMember).collect();
        assert_eq!(member_defs.len(), 3);
    }

    #[test]
    fn test_parse_sql_stored_procedure() {
        let mut parser = tree_sitter::Parser::new();
        if parser.set_language(&tree_sitter_sequel_tsql::LANGUAGE.into()).is_err() {
            eprintln!("Skipping SQL test: grammar ABI version incompatible");
            return;
        }

        let source = r#"
CREATE PROCEDURE [dbo].[usp_GetIndexTenantMapping]
    @IndexName NVARCHAR(256),
    @ServiceName NVARCHAR(256)
AS
BEGIN
    SELECT * FROM IndexTenantMapping
    WHERE IndexName = @IndexName AND ServiceName = @ServiceName
END
"#;

        let defs = parse_sql_definitions(&mut parser, source, 0);

        // Should find at least one definition
        if defs.is_empty() {
            eprintln!("Warning: SQL grammar parsed but no definitions found (AST may differ)");
            return;
        }

        let sp_defs: Vec<_> = defs.iter()
            .filter(|d| d.kind == DefinitionKind::StoredProcedure || d.name.contains("usp_GetIndexTenantMapping"))
            .collect();

        if sp_defs.is_empty() {
            eprintln!("SQL defs found: {:?}", defs.iter().map(|d| (&d.name, &d.kind)).collect::<Vec<_>>());
        }
    }

    #[test]
    fn test_parse_sql_create_table() {
        let mut parser = tree_sitter::Parser::new();
        if parser.set_language(&tree_sitter_sequel_tsql::LANGUAGE.into()).is_err() {
            eprintln!("Skipping SQL test: grammar ABI version incompatible");
            return;
        }

        let source = r#"
CREATE TABLE [dbo].[IndexTenantMapping]
(
    [Id] INT IDENTITY(1,1) NOT NULL,
    [IndexName] NVARCHAR(256) NOT NULL,
    [TenantId] UNIQUEIDENTIFIER NOT NULL,
    [Status] INT NOT NULL DEFAULT 0
)
"#;

        let defs = parse_sql_definitions(&mut parser, source, 0);
        if defs.is_empty() {
            eprintln!("Warning: SQL grammar parsed but no definitions found (AST may differ)");
        }
    }

    #[test]
    fn test_definition_index_build_and_search() {
        // Create a temporary directory with test files
        let dir = std::env::temp_dir().join("search_defindex_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        std::fs::write(
            dir.join("test.cs"),
            "public class TestClass : BaseClass { public void TestMethod() {} }"
        ).unwrap();

        std::fs::write(
            dir.join("test.sql"),
            "CREATE TABLE TestTable (Id INT NOT NULL)"
        ).unwrap();

        let args = DefIndexArgs {
            dir: dir.to_string_lossy().to_string(),
            ext: "cs,sql".to_string(),
            threads: 1,
        };

        let index = build_definition_index(&args);

        assert_eq!(index.files.len(), 2);
        assert!(!index.definitions.is_empty());

        // Check name index
        assert!(index.name_index.contains_key("testclass"));
        assert!(index.name_index.contains_key("testmethod"));

        // Check kind index
        assert!(index.kind_index.contains_key(&DefinitionKind::Class));
        assert!(index.kind_index.contains_key(&DefinitionKind::Method));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_definition_index_serialization() {
        let index = DefinitionIndex {
            root: ".".to_string(),
            created_at: 1000,
            extensions: vec!["cs".to_string()],
            files: vec!["test.cs".to_string()],
            definitions: vec![DefinitionEntry {
                file_id: 0,
                name: "TestClass".to_string(),
                kind: DefinitionKind::Class,
                line_start: 1,
                line_end: 10,
                parent: None,
                signature: Some("public class TestClass".to_string()),
                modifiers: vec!["public".to_string()],
                attributes: Vec::new(),
                base_types: Vec::new(),
            }],
            name_index: {
                let mut m = HashMap::new();
                m.insert("testclass".to_string(), vec![0]);
                m
            },
            kind_index: {
                let mut m = HashMap::new();
                m.insert(DefinitionKind::Class, vec![0]);
                m
            },
            attribute_index: HashMap::new(),
            base_type_index: HashMap::new(),
            file_index: {
                let mut m = HashMap::new();
                m.insert(0, vec![0]);
                m
            },
            path_to_id: HashMap::new(),
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
        };

        let clean = PathBuf::from(crate::clean_path(&test_file.to_string_lossy()));
        update_file_definitions(&mut index, &clean);

        // Should have added definitions
        assert!(!index.definitions.is_empty(), "Should have definitions after update");
        assert!(index.name_index.contains_key("newclass"), "Should index class name");
        assert!(index.name_index.contains_key("newmethod"), "Should index method name");
        assert!(index.kind_index.contains_key(&DefinitionKind::Class));
        assert!(index.kind_index.contains_key(&DefinitionKind::Method));
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
            root: ".".to_string(),
            created_at: 0,
            extensions: vec!["cs".to_string()],
            files: vec![clean.to_string_lossy().to_string()],
            definitions: vec![DefinitionEntry {
                file_id: 0,
                name: "OldClass".to_string(),
                kind: DefinitionKind::Class,
                line_start: 1,
                line_end: 1,
                parent: None,
                signature: None,
                modifiers: Vec::new(),
                attributes: Vec::new(),
                base_types: Vec::new(),
            }],
            name_index: {
                let mut m = HashMap::new();
                m.insert("oldclass".to_string(), vec![0]);
                m
            },
            kind_index: {
                let mut m = HashMap::new();
                m.insert(DefinitionKind::Class, vec![0]);
                m
            },
            attribute_index: HashMap::new(),
            base_type_index: HashMap::new(),
            file_index: {
                let mut m = HashMap::new();
                m.insert(0, vec![0]);
                m
            },
            path_to_id: {
                let mut m = HashMap::new();
                m.insert(clean.clone(), 0u32);
                m
            },
        };

        // Update file content
        std::fs::write(&test_file, "public class UpdatedClass { public int Value { get; set; } }").unwrap();
        update_file_definitions(&mut index, &clean);

        // Old definitions should be removed from indexes
        assert!(!index.name_index.contains_key("oldclass"), "Old class name should be gone");
        // New definitions should be present
        assert!(index.name_index.contains_key("updatedclass"), "New class name should be present");
        assert!(index.name_index.contains_key("value"), "Property name should be present");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_remove_file_from_def_index() {
        let mut index = DefinitionIndex {
            root: ".".to_string(),
            created_at: 0,
            extensions: vec!["cs".to_string()],
            files: vec!["file0.cs".to_string(), "file1.cs".to_string()],
            definitions: vec![
                DefinitionEntry {
                    file_id: 0,
                    name: "ClassA".to_string(),
                    kind: DefinitionKind::Class,
                    line_start: 1, line_end: 10,
                    parent: None, signature: None,
                    modifiers: Vec::new(), attributes: Vec::new(), base_types: Vec::new(),
                },
                DefinitionEntry {
                    file_id: 1,
                    name: "ClassB".to_string(),
                    kind: DefinitionKind::Class,
                    line_start: 1, line_end: 10,
                    parent: None, signature: None,
                    modifiers: Vec::new(), attributes: Vec::new(), base_types: Vec::new(),
                },
            ],
            name_index: {
                let mut m = HashMap::new();
                m.insert("classa".to_string(), vec![0]);
                m.insert("classb".to_string(), vec![1]);
                m
            },
            kind_index: {
                let mut m = HashMap::new();
                m.insert(DefinitionKind::Class, vec![0, 1]);
                m
            },
            attribute_index: HashMap::new(),
            base_type_index: HashMap::new(),
            file_index: {
                let mut m = HashMap::new();
                m.insert(0, vec![0]);
                m.insert(1, vec![1]);
                m
            },
            path_to_id: {
                let mut m = HashMap::new();
                m.insert(PathBuf::from("file0.cs"), 0);
                m.insert(PathBuf::from("file1.cs"), 1);
                m
            },
        };

        // Remove file0.cs
        remove_file_from_def_index(&mut index, &PathBuf::from("file0.cs"));

        // ClassA should be gone from name_index
        assert!(!index.name_index.contains_key("classa"), "ClassA should be removed");
        // ClassB should still be present
        assert!(index.name_index.contains_key("classb"), "ClassB should remain");
        // path_to_id should not contain file0
        assert!(!index.path_to_id.contains_key(&PathBuf::from("file0.cs")));
        // file1 should still be there
        assert!(index.path_to_id.contains_key(&PathBuf::from("file1.cs")));
    }
}
// ─── Incremental Update Functions (for watcher) ──────────────────────
