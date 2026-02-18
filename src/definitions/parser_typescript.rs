//! TypeScript AST parser using tree-sitter: extracts definitions.

use super::types::*;

// ─── Main entry point ───────────────────────────────────────────────

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
    walk_typescript_node_collecting(tree.root_node(), source, file_id, None, &mut defs);

    // Call sites are deferred for TypeScript — always return empty
    (defs, Vec::new())
}

// ─── AST walking ────────────────────────────────────────────────────

fn walk_typescript_node_collecting(
    node: tree_sitter::Node,
    source: &str,
    file_id: u32,
    parent_name: Option<&str>,
    defs: &mut Vec<DefinitionEntry>,
) {
    let kind = node.kind();

    match kind {
        "class_declaration" | "abstract_class_declaration" => {
            if let Some(def) = extract_ts_class_def(node, source, file_id, parent_name) {
                let name = def.name.clone();
                defs.push(def);
                // Walk into class body
                if let Some(body) = find_child_by_kind(node, "class_body") {
                    for i in 0..body.child_count() {
                        if let Some(child) = body.child(i) {
                            walk_typescript_node_collecting(child, source, file_id, Some(&name), defs);
                        }
                    }
                }
                return;
            }
        }
        "interface_declaration" => {
            if let Some(def) = extract_ts_interface_def(node, source, file_id, parent_name) {
                let name = def.name.clone();
                defs.push(def);
                // Walk into interface body for property signatures
                if let Some(body) = find_child_by_kind(node, "object_type")
                    .or_else(|| find_child_by_kind(node, "interface_body"))
                {
                    for i in 0..body.child_count() {
                        if let Some(child) = body.child(i) {
                            walk_typescript_node_collecting(child, source, file_id, Some(&name), defs);
                        }
                    }
                }
                return;
            }
        }
        "enum_declaration" => {
            if let Some(def) = extract_ts_enum_def(node, source, file_id, parent_name) {
                let name = def.name.clone();
                defs.push(def);
                // Walk into enum body for members
                if let Some(body) = find_child_by_kind(node, "enum_body") {
                    for i in 0..body.child_count() {
                        if let Some(child) = body.child(i) {
                            walk_typescript_node_collecting(child, source, file_id, Some(&name), defs);
                        }
                    }
                }
                return;
            }
        }
        "function_declaration" => {
            if let Some(def) = extract_ts_function_def(node, source, file_id, parent_name) {
                defs.push(def);
                return;
            }
        }
        "method_definition" => {
            if let Some(def) = extract_ts_method_def(node, source, file_id, parent_name) {
                defs.push(def);
                return;
            }
        }
        "abstract_method_signature" => {
            if let Some(def) = extract_ts_abstract_method_sig(node, source, file_id, parent_name) {
                defs.push(def);
                return;
            }
        }
        "method_signature" => {
            if let Some(def) = extract_ts_method_signature(node, source, file_id, parent_name) {
                defs.push(def);
                return;
            }
        }
        "public_field_definition" => {
            if let Some(def) = extract_ts_field_def(node, source, file_id, parent_name) {
                defs.push(def);
                return;
            }
        }
        "property_signature" => {
            if let Some(def) = extract_ts_property_signature(node, source, file_id, parent_name) {
                defs.push(def);
                return;
            }
        }
        "type_alias_declaration" => {
            if let Some(def) = extract_ts_type_alias_def(node, source, file_id, parent_name) {
                defs.push(def);
                return;
            }
        }
        "lexical_declaration" => {
            // Only extract exported variable declarations
            if is_exported(node) {
                extract_ts_variable_defs(node, source, file_id, parent_name, defs);
                return;
            }
        }
        "enum_member" => {
            if let Some(def) = extract_ts_enum_member(node, source, file_id, parent_name) {
                defs.push(def);
                return;
            }
        }
        // In tree-sitter-typescript, enum members can also be plain property_identifier
        // nodes inside enum_body (without an enum_member wrapper)
        "property_identifier" if is_inside_enum_body(node) => {
            let name = node_text(node, source).to_string();
            if !name.is_empty() {
                defs.push(DefinitionEntry {
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
                });
                return;
            }
        }
        // For export_statement, walk into the child declaration
        "export_statement" => {
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    walk_typescript_node_collecting(child, source, file_id, parent_name, defs);
                }
            }
            return;
        }
        _ => {}
    }

    // Default: recurse into children
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            walk_typescript_node_collecting(child, source, file_id, parent_name, defs);
        }
    }
}

// ─── Helper utilities ───────────────────────────────────────────────

fn node_text<'a>(node: tree_sitter::Node, source: &'a str) -> &'a str {
    node.utf8_text(source.as_bytes()).unwrap_or("")
}

fn find_child_by_kind<'a>(node: tree_sitter::Node<'a>, kind: &str) -> Option<tree_sitter::Node<'a>> {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == kind {
                return Some(child);
            }
        }
    }
    None
}

fn find_child_by_field<'a>(node: tree_sitter::Node<'a>, field: &str) -> Option<tree_sitter::Node<'a>> {
    node.child_by_field_name(field)
}

/// Check if a node is exported (its parent is an export_statement).
fn is_exported(node: tree_sitter::Node) -> bool {
    if let Some(parent) = node.parent() {
        return parent.kind() == "export_statement";
    }
    false
}

/// Extract modifiers from a TypeScript node.
/// Handles: accessibility_modifier (public/private/protected), static, async,
/// abstract, readonly, export, override.
fn extract_ts_modifiers(node: tree_sitter::Node, source: &str) -> Vec<String> {
    let mut modifiers = Vec::new();
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            match child.kind() {
                "accessibility_modifier" => {
                    modifiers.push(node_text(child, source).to_string());
                }
                "static" | "async" | "abstract" | "readonly" | "override" | "declare" | "const" => {
                    modifiers.push(node_text(child, source).to_string());
                }
                _ => {}
            }
        }
    }
    // Check if exported
    if is_exported(node) {
        modifiers.push("export".to_string());
    }
    modifiers
}

/// Extract decorators from a TypeScript node (equivalent to C# attributes).
fn extract_ts_decorators(node: tree_sitter::Node, source: &str) -> Vec<String> {
    let mut decorators = Vec::new();
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "decorator" {
                // Get full decorator text minus the leading '@'
                let text = node_text(child, source);
                let trimmed = text.strip_prefix('@').unwrap_or(text).to_string();
                decorators.push(trimmed);
            }
        }
    }
    decorators
}

/// Extract base types / heritage (extends/implements) from a class or interface.
fn extract_ts_heritage(node: tree_sitter::Node, source: &str) -> Vec<String> {
    let mut base_types = Vec::new();
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            match child.kind() {
                "class_heritage" | "extends_clause" | "implements_clause"
                | "extends_type_clause" => {
                    // Walk the clause children to find type identifiers
                    for j in 0..child.child_count() {
                        if let Some(type_node) = child.child(j) {
                            match type_node.kind() {
                                // In class_heritage, there may be nested extends_clause/implements_clause
                                "extends_clause" | "implements_clause" => {
                                    for k in 0..type_node.child_count() {
                                        if let Some(t) = type_node.child(k) {
                                            if t.is_named() && t.kind() != "extends" && t.kind() != "implements" {
                                                base_types.push(node_text(t, source).to_string());
                                            }
                                        }
                                    }
                                }
                                _ if type_node.is_named()
                                    && type_node.kind() != "extends"
                                    && type_node.kind() != "implements" =>
                                {
                                    base_types.push(node_text(type_node, source).to_string());
                                }
                                _ => {}
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    base_types
}

/// Extract type annotation string from a node (looks for type_annotation child).
fn extract_type_annotation(node: tree_sitter::Node, source: &str) -> Option<String> {
    find_child_by_kind(node, "type_annotation").map(|ta| {
        // type_annotation is ": Type", we want the Type part
        let text = node_text(ta, source).trim();
        // Strip leading ':'
        text.strip_prefix(':').unwrap_or(text).trim().to_string()
    })
}

/// Extract formal parameters text from a function/method node.
fn extract_params_text(node: tree_sitter::Node, source: &str) -> Option<String> {
    find_child_by_kind(node, "formal_parameters").map(|params| {
        node_text(params, source).to_string()
    })
}

/// Build a signature for a function/method-like declaration.
fn build_function_signature(
    name: &str,
    params: Option<&str>,
    return_type: Option<&str>,
    prefix_modifiers: &[String],
) -> String {
    let mut sig = String::new();
    for m in prefix_modifiers {
        if matches!(m.as_str(), "async" | "static" | "abstract" | "export") {
            sig.push_str(m);
            sig.push(' ');
        }
    }
    sig.push_str(name);
    if let Some(p) = params {
        sig.push_str(p);
    } else {
        sig.push_str("()");
    }
    if let Some(rt) = return_type {
        sig.push_str(": ");
        sig.push_str(rt);
    }
    sig
}

// ─── Definition extraction helpers ──────────────────────────────────

fn extract_ts_class_def(
    node: tree_sitter::Node,
    source: &str,
    file_id: u32,
    parent_name: Option<&str>,
) -> Option<DefinitionEntry> {
    let name_node = find_child_by_field(node, "name")?;
    let name = node_text(name_node, source).to_string();
    let mut modifiers = extract_ts_modifiers(node, source);
    let decorators = extract_ts_decorators(node, source);
    let base_types = extract_ts_heritage(node, source);

    // Add "abstract" for abstract_class_declaration if not already present
    if node.kind() == "abstract_class_declaration" && !modifiers.contains(&"abstract".to_string()) {
        modifiers.push("abstract".to_string());
    }

    // Build signature: everything up to the class body
    let sig = build_type_signature(node, source);

    Some(DefinitionEntry {
        file_id,
        name,
        kind: DefinitionKind::Class,
        line_start: node.start_position().row as u32 + 1,
        line_end: node.end_position().row as u32 + 1,
        parent: parent_name.map(|s| s.to_string()),
        signature: Some(sig),
        modifiers,
        attributes: decorators,
        base_types,
    })
}

fn extract_ts_interface_def(
    node: tree_sitter::Node,
    source: &str,
    file_id: u32,
    parent_name: Option<&str>,
) -> Option<DefinitionEntry> {
    let name_node = find_child_by_field(node, "name")?;
    let name = node_text(name_node, source).to_string();
    let modifiers = extract_ts_modifiers(node, source);
    let base_types = extract_ts_heritage(node, source);
    let sig = build_type_signature(node, source);

    Some(DefinitionEntry {
        file_id,
        name,
        kind: DefinitionKind::Interface,
        line_start: node.start_position().row as u32 + 1,
        line_end: node.end_position().row as u32 + 1,
        parent: parent_name.map(|s| s.to_string()),
        signature: Some(sig),
        modifiers,
        attributes: Vec::new(),
        base_types,
    })
}

fn extract_ts_enum_def(
    node: tree_sitter::Node,
    source: &str,
    file_id: u32,
    parent_name: Option<&str>,
) -> Option<DefinitionEntry> {
    let name_node = find_child_by_field(node, "name")?;
    let name = node_text(name_node, source).to_string();
    let modifiers = extract_ts_modifiers(node, source);
    let sig = build_type_signature(node, source);

    Some(DefinitionEntry {
        file_id,
        name,
        kind: DefinitionKind::Enum,
        line_start: node.start_position().row as u32 + 1,
        line_end: node.end_position().row as u32 + 1,
        parent: parent_name.map(|s| s.to_string()),
        signature: Some(sig),
        modifiers,
        attributes: Vec::new(),
        base_types: Vec::new(),
    })
}

fn extract_ts_function_def(
    node: tree_sitter::Node,
    source: &str,
    file_id: u32,
    parent_name: Option<&str>,
) -> Option<DefinitionEntry> {
    let name_node = find_child_by_field(node, "name")?;
    let name = node_text(name_node, source).to_string();
    let modifiers = extract_ts_modifiers(node, source);
    let decorators = extract_ts_decorators(node, source);
    let params = extract_params_text(node, source);
    let return_type = extract_type_annotation(node, source);
    let sig = build_function_signature(
        &name,
        params.as_deref(),
        return_type.as_deref(),
        &modifiers,
    );

    Some(DefinitionEntry {
        file_id,
        name,
        kind: DefinitionKind::Function,
        line_start: node.start_position().row as u32 + 1,
        line_end: node.end_position().row as u32 + 1,
        parent: parent_name.map(|s| s.to_string()),
        signature: Some(sig),
        modifiers,
        attributes: decorators,
        base_types: Vec::new(),
    })
}

fn extract_ts_method_def(
    node: tree_sitter::Node,
    source: &str,
    file_id: u32,
    parent_name: Option<&str>,
) -> Option<DefinitionEntry> {
    let name_node = find_child_by_field(node, "name")?;
    let name = node_text(name_node, source).to_string();

    // Detect constructor
    let is_constructor = name == "constructor";
    let kind = if is_constructor {
        DefinitionKind::Constructor
    } else {
        DefinitionKind::Method
    };

    let modifiers = extract_ts_modifiers(node, source);
    let decorators = extract_ts_decorators(node, source);
    let params = extract_params_text(node, source);
    let return_type = extract_type_annotation(node, source);
    let sig = build_function_signature(
        &name,
        params.as_deref(),
        return_type.as_deref(),
        &modifiers,
    );

    Some(DefinitionEntry {
        file_id,
        name,
        kind,
        line_start: node.start_position().row as u32 + 1,
        line_end: node.end_position().row as u32 + 1,
        parent: parent_name.map(|s| s.to_string()),
        signature: Some(sig),
        modifiers,
        attributes: decorators,
        base_types: Vec::new(),
    })
}

fn extract_ts_field_def(
    node: tree_sitter::Node,
    source: &str,
    file_id: u32,
    parent_name: Option<&str>,
) -> Option<DefinitionEntry> {
    let name_node = find_child_by_field(node, "name")
        .or_else(|| find_child_by_kind(node, "property_identifier"))?;
    let name = node_text(name_node, source).to_string();
    let modifiers = extract_ts_modifiers(node, source);
    let decorators = extract_ts_decorators(node, source);
    let type_ann = extract_type_annotation(node, source);
    let sig = if let Some(ref t) = type_ann {
        format!("{}: {}", name, t)
    } else {
        name.clone()
    };

    Some(DefinitionEntry {
        file_id,
        name,
        kind: DefinitionKind::Field,
        line_start: node.start_position().row as u32 + 1,
        line_end: node.end_position().row as u32 + 1,
        parent: parent_name.map(|s| s.to_string()),
        signature: Some(sig),
        modifiers,
        attributes: decorators,
        base_types: Vec::new(),
    })
}

fn extract_ts_property_signature(
    node: tree_sitter::Node,
    source: &str,
    file_id: u32,
    parent_name: Option<&str>,
) -> Option<DefinitionEntry> {
    let name_node = find_child_by_field(node, "name")
        .or_else(|| find_child_by_kind(node, "property_identifier"))?;
    let name = node_text(name_node, source).to_string();
    let mut modifiers = Vec::new();
    // Check for readonly
    if find_child_by_kind(node, "readonly").is_some() {
        modifiers.push("readonly".to_string());
    }
    let type_ann = extract_type_annotation(node, source);
    let sig = if let Some(ref t) = type_ann {
        format!("{}: {}", name, t)
    } else {
        name.clone()
    };

    Some(DefinitionEntry {
        file_id,
        name,
        kind: DefinitionKind::Property,
        line_start: node.start_position().row as u32 + 1,
        line_end: node.end_position().row as u32 + 1,
        parent: parent_name.map(|s| s.to_string()),
        signature: Some(sig),
        modifiers,
        attributes: Vec::new(),
        base_types: Vec::new(),
    })
}

fn extract_ts_type_alias_def(
    node: tree_sitter::Node,
    source: &str,
    file_id: u32,
    parent_name: Option<&str>,
) -> Option<DefinitionEntry> {
    let name_node = find_child_by_field(node, "name")?;
    let name = node_text(name_node, source).to_string();
    let modifiers = extract_ts_modifiers(node, source);

    // Build signature from the full type alias text (excluding body/semicolon)
    let sig = {
        let text = node_text(node, source);
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    };

    Some(DefinitionEntry {
        file_id,
        name,
        kind: DefinitionKind::TypeAlias,
        line_start: node.start_position().row as u32 + 1,
        line_end: node.end_position().row as u32 + 1,
        parent: parent_name.map(|s| s.to_string()),
        signature: Some(sig),
        modifiers,
        attributes: Vec::new(),
        base_types: Vec::new(),
    })
}

fn extract_ts_variable_defs(
    node: tree_sitter::Node,
    source: &str,
    file_id: u32,
    parent_name: Option<&str>,
    defs: &mut Vec<DefinitionEntry>,
) {
    // lexical_declaration contains "const"/"let" keyword and variable_declarator(s)
    let mut decl_keyword = String::new();
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "const" || child.kind() == "let" || child.kind() == "var" {
                decl_keyword = node_text(child, source).to_string();
            }
        }
    }

    let mut modifiers = vec![];
    if !decl_keyword.is_empty() {
        modifiers.push(decl_keyword.clone());
    }
    if is_exported(node) {
        modifiers.push("export".to_string());
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "variable_declarator" {
                if let Some(name_node) = find_child_by_field(child, "name") {
                    let name = node_text(name_node, source).to_string();
                    let type_ann = extract_type_annotation(child, source);
                    let sig = if let Some(ref t) = type_ann {
                        format!("{} {}: {}", decl_keyword, name, t)
                    } else {
                        format!("{} {}", decl_keyword, name)
                    };

                    defs.push(DefinitionEntry {
                        file_id,
                        name,
                        kind: DefinitionKind::Variable,
                        line_start: node.start_position().row as u32 + 1,
                        line_end: node.end_position().row as u32 + 1,
                        parent: parent_name.map(|s| s.to_string()),
                        signature: Some(sig.trim().to_string()),
                        modifiers: modifiers.clone(),
                        attributes: Vec::new(),
                        base_types: Vec::new(),
                    });
                }
            }
        }
    }
}

fn extract_ts_enum_member(
    node: tree_sitter::Node,
    source: &str,
    file_id: u32,
    parent_name: Option<&str>,
) -> Option<DefinitionEntry> {
    let name_node = find_child_by_field(node, "name")
        .or_else(|| find_child_by_kind(node, "property_identifier"))?;
    let name = node_text(name_node, source).to_string();

    // Check for initializer
    let sig = {
        let text = node_text(node, source).trim().to_string();
        if text.is_empty() { None } else { Some(text) }
    };

    Some(DefinitionEntry {
        file_id,
        name,
        kind: DefinitionKind::EnumMember,
        line_start: node.start_position().row as u32 + 1,
        line_end: node.end_position().row as u32 + 1,
        parent: parent_name.map(|s| s.to_string()),
        signature: sig,
        modifiers: Vec::new(),
        attributes: Vec::new(),
        base_types: Vec::new(),
    })
}

/// Build a type signature from everything before the body (class_body, object_type, enum_body).
fn build_type_signature(node: tree_sitter::Node, source: &str) -> String {
    let start = node.start_byte();
    let mut end = node.end_byte();
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            match child.kind() {
                "class_body" | "object_type" | "interface_body" | "enum_body" | "{" => {
                    end = child.start_byte();
                    break;
                }
                _ => {}
            }
        }
    }
    let text = &source[start..end];
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Check if a node is directly inside an enum_body (its parent is enum_body).
fn is_inside_enum_body(node: tree_sitter::Node) -> bool {
    if let Some(parent) = node.parent() {
        return parent.kind() == "enum_body";
    }
    false
}

/// Extract an abstract method signature (e.g., `abstract handle(): void;`).
fn extract_ts_abstract_method_sig(
    node: tree_sitter::Node,
    source: &str,
    file_id: u32,
    parent_name: Option<&str>,
) -> Option<DefinitionEntry> {
    let name_node = find_child_by_kind(node, "property_identifier")?;
    let name = node_text(name_node, source).to_string();
    let mut modifiers = extract_ts_modifiers(node, source);
    if !modifiers.contains(&"abstract".to_string()) {
        modifiers.push("abstract".to_string());
    }
    let params = extract_params_text(node, source);
    let return_type = extract_type_annotation(node, source);
    let sig = build_function_signature(&name, params.as_deref(), return_type.as_deref(), &modifiers);

    Some(DefinitionEntry {
        file_id,
        name,
        kind: DefinitionKind::Method,
        line_start: node.start_position().row as u32 + 1,
        line_end: node.end_position().row as u32 + 1,
        parent: parent_name.map(|s| s.to_string()),
        signature: Some(sig),
        modifiers,
        attributes: Vec::new(),
        base_types: Vec::new(),
    })
}

/// Extract a method signature from an interface body (e.g., `process(order: Order): Promise<void>;`).
fn extract_ts_method_signature(
    node: tree_sitter::Node,
    source: &str,
    file_id: u32,
    parent_name: Option<&str>,
) -> Option<DefinitionEntry> {
    let name_node = find_child_by_kind(node, "property_identifier")?;
    let name = node_text(name_node, source).to_string();
    let modifiers = extract_ts_modifiers(node, source);
    let params = extract_params_text(node, source);
    let return_type = extract_type_annotation(node, source);
    let sig = build_function_signature(&name, params.as_deref(), return_type.as_deref(), &modifiers);

    Some(DefinitionEntry {
        file_id,
        name,
        kind: DefinitionKind::Property,
        line_start: node.start_position().row as u32 + 1,
        line_end: node.end_position().row as u32 + 1,
        parent: parent_name.map(|s| s.to_string()),
        signature: Some(sig),
        modifiers,
        attributes: Vec::new(),
        base_types: Vec::new(),
    })
}