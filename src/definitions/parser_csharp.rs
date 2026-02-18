//! C# AST parser using tree-sitter: extracts definitions and call sites.

use std::collections::HashMap;

use super::types::*;

// ─── Main entry point ───────────────────────────────────────────────

pub(crate) fn parse_csharp_definitions(
    parser: &mut tree_sitter::Parser,
    source: &str,
    file_id: u32,
) -> (Vec<DefinitionEntry>, Vec<(usize, Vec<CallSite>)>) {
    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => {
            eprintln!("[def-index] WARNING: tree-sitter C# parse returned None for file_id={}", file_id);
            return (Vec::new(), Vec::new());
        }
    };

    let mut defs = Vec::new();
    let source_bytes = source.as_bytes();
    let mut method_nodes: Vec<(usize, tree_sitter::Node)> = Vec::new();
    walk_csharp_node_collecting(tree.root_node(), source_bytes, file_id, None, &mut defs, &mut method_nodes);

    // Build per-class field type maps from the collected defs
    let mut class_field_types: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut class_base_types: HashMap<String, Vec<String>> = HashMap::new();

    for def in &defs {
        if let Some(ref parent) = def.parent {
            match def.kind {
                DefinitionKind::Field | DefinitionKind::Property => {
                    if let Some(ref sig) = def.signature {
                        if let Some((type_name, _field_name)) = parse_field_signature(sig) {
                            class_field_types
                                .entry(parent.clone())
                                .or_default()
                                .insert(def.name.clone(), type_name);
                        }
                    }
                }
                DefinitionKind::Class | DefinitionKind::Struct | DefinitionKind::Record => {
                    if !def.base_types.is_empty() {
                        class_base_types.insert(def.name.clone(), def.base_types.clone());
                    }
                }
                _ => {}
            }
        }
        if def.parent.is_none() && matches!(def.kind, DefinitionKind::Class | DefinitionKind::Struct | DefinitionKind::Record) {
            if !def.base_types.is_empty() {
                class_base_types.insert(def.name.clone(), def.base_types.clone());
            }
        }
    }

    // Extract constructor parameter types as field types (DI pattern)
    for def in &defs {
        if def.kind == DefinitionKind::Constructor {
            if let Some(ref parent) = def.parent {
                if let Some(ref sig) = def.signature {
                    let param_types = extract_constructor_param_types(sig);
                    let field_map = class_field_types.entry(parent.clone()).or_default();
                    for (param_name, param_type) in param_types {
                        let underscore_name = format!("_{}", param_name);
                        if !field_map.contains_key(&underscore_name) {
                            field_map.insert(underscore_name, param_type.clone());
                        }
                        if !field_map.contains_key(&param_name) {
                            field_map.insert(param_name, param_type);
                        }
                    }
                }
            }
        }
    }

    // Extract call sites from pre-collected method nodes
    let mut call_sites: Vec<(usize, Vec<CallSite>)> = Vec::new();
    for &(def_local_idx, method_node) in &method_nodes {
        let def = &defs[def_local_idx];
        let parent_name = def.parent.as_deref().unwrap_or("");
        let field_types = class_field_types.get(parent_name)
            .cloned()
            .unwrap_or_default();
        let base_types = class_base_types.get(parent_name)
            .cloned()
            .unwrap_or_default();

        let calls = extract_call_sites(method_node, source_bytes, parent_name, &field_types, &base_types);
        if !calls.is_empty() {
            call_sites.push((def_local_idx, calls));
        }
    }

    (defs, call_sites)
}

// ─── Field/Constructor signature parsing ────────────────────────────

/// Parse a field/property signature like "IUserService _userService" into (type, name)
pub(crate) fn parse_field_signature(sig: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = sig.trim().rsplitn(2, char::is_whitespace).collect();
    if parts.len() == 2 {
        let field_name = parts[0].trim().to_string();
        let type_name = parts[1].trim().to_string();
        let base_type = type_name.split('<').next().unwrap_or(&type_name).to_string();
        if !base_type.is_empty() && !field_name.is_empty() {
            return Some((base_type, field_name));
        }
    }
    None
}

/// Extract parameter names and types from a constructor signature.
pub(crate) fn extract_constructor_param_types(sig: &str) -> Vec<(String, String)> {
    let mut result = Vec::new();
    let start = match sig.find('(') {
        Some(i) => i + 1,
        None => return result,
    };
    let end = match sig.rfind(')') {
        Some(i) => i,
        None => return result,
    };
    if start >= end { return result; }

    let params_str = &sig[start..end];
    for param in params_str.split(',') {
        let param = param.trim();
        if param.is_empty() { continue; }
        let parts: Vec<&str> = param.split_whitespace().collect();
        if parts.len() >= 2 {
            let name = parts[parts.len() - 1];
            let type_parts: Vec<&&str> = parts[..parts.len() - 1].iter()
                .filter(|p| !matches!(**p, "ref" | "out" | "in" | "params" | "this"))
                .collect();
            if let Some(type_str) = type_parts.last() {
                let base_type = type_str.split('<').next().unwrap_or(type_str);
                result.push((name.to_string(), base_type.to_string()));
            }
        }
    }
    result
}

// ─── Call site extraction ───────────────────────────────────────────

fn extract_call_sites(
    method_node: tree_sitter::Node,
    source: &[u8],
    class_name: &str,
    field_types: &HashMap<String, String>,
    base_types: &[String],
) -> Vec<CallSite> {
    let mut calls = Vec::new();

    let body = find_child_by_kind(method_node, "block")
        .or_else(|| find_child_by_kind(method_node, "arrow_expression_clause"));

    if let Some(body_node) = body {
        walk_for_invocations(body_node, source, class_name, field_types, base_types, &mut calls);
    }

    calls.sort_by(|a, b| a.line.cmp(&b.line)
        .then_with(|| a.method_name.cmp(&b.method_name))
        .then_with(|| a.receiver_type.cmp(&b.receiver_type)));
    calls.dedup_by(|a, b| a.line == b.line && a.method_name == b.method_name && a.receiver_type == b.receiver_type);

    calls
}

fn walk_for_invocations(
    node: tree_sitter::Node,
    source: &[u8],
    class_name: &str,
    field_types: &HashMap<String, String>,
    base_types: &[String],
    calls: &mut Vec<CallSite>,
) {
    match node.kind() {
        "invocation_expression" => {
            if let Some(call) = extract_invocation(node, source, class_name, field_types, base_types) {
                calls.push(call);
            }
            for i in 0..node.child_count() {
                let child = node.child(i).unwrap();
                if child.kind() == "argument_list" {
                    walk_for_invocations(child, source, class_name, field_types, base_types, calls);
                }
            }
            return;
        }
        "object_creation_expression" => {
            if let Some(call) = extract_object_creation(node, source) {
                calls.push(call);
            }
            for i in 0..node.child_count() {
                let child = node.child(i).unwrap();
                if child.kind() == "argument_list" {
                    walk_for_invocations(child, source, class_name, field_types, base_types, calls);
                }
            }
            return;
        }
        _ => {}
    }

    for i in 0..node.child_count() {
        walk_for_invocations(node.child(i).unwrap(), source, class_name, field_types, base_types, calls);
    }
}

fn extract_invocation(
    node: tree_sitter::Node,
    source: &[u8],
    class_name: &str,
    field_types: &HashMap<String, String>,
    base_types: &[String],
) -> Option<CallSite> {
    let expr = node.child(0)?;
    let line = node.start_position().row as u32 + 1;

    match expr.kind() {
        "identifier" => {
            let method_name = node_text(expr, source).to_string();
            Some(CallSite { method_name, receiver_type: None, line })
        }
        "member_access_expression" => {
            extract_member_access_call(expr, source, class_name, field_types, base_types, line)
        }
        "conditional_access_expression" => {
            extract_conditional_access_call(expr, source, class_name, field_types, base_types, line)
        }
        "generic_name" => {
            let name_node = find_child_by_field(expr, "name")
                .or_else(|| expr.child(0));
            let method_name = name_node.map(|n| node_text(n, source)).unwrap_or("");
            if !method_name.is_empty() {
                Some(CallSite { method_name: method_name.to_string(), receiver_type: None, line })
            } else {
                None
            }
        }
        _ => None,
    }
}

fn extract_member_access_call(
    node: tree_sitter::Node,
    source: &[u8],
    class_name: &str,
    field_types: &HashMap<String, String>,
    base_types: &[String],
    line: u32,
) -> Option<CallSite> {
    let name_node = find_child_by_field(node, "name")?;
    let method_name = node_text(name_node, source).to_string();

    let receiver_node = find_child_by_field(node, "expression")
        .or_else(|| node.child(0))?;
    let receiver_type = resolve_receiver_type(receiver_node, source, class_name, field_types, base_types);

    Some(CallSite { method_name, receiver_type, line })
}

fn extract_conditional_access_call(
    node: tree_sitter::Node,
    source: &[u8],
    class_name: &str,
    field_types: &HashMap<String, String>,
    base_types: &[String],
    line: u32,
) -> Option<CallSite> {
    let receiver_node = node.child(0)?;

    let mut binding = None;
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        if child.kind() == "member_binding_expression" {
            binding = Some(child);
            break;
        }
    }

    let binding = binding?;
    let name_node = find_child_by_field(binding, "name")
        .or_else(|| binding.child(binding.child_count().saturating_sub(1)))?;
    let method_name = node_text(name_node, source).to_string();

    let receiver_type = resolve_receiver_type(receiver_node, source, class_name, field_types, base_types);

    Some(CallSite { method_name, receiver_type, line })
}

fn extract_object_creation(
    node: tree_sitter::Node,
    source: &[u8],
) -> Option<CallSite> {
    let type_node = find_child_by_field(node, "type")?;
    let type_text = node_text(type_node, source);
    let type_name = type_text.split('<').next().unwrap_or(type_text).trim();

    if type_name.is_empty() { return None; }

    Some(CallSite {
        method_name: type_name.to_string(),
        receiver_type: Some(type_name.to_string()),
        line: node.start_position().row as u32 + 1,
    })
}

fn resolve_receiver_type(
    receiver: tree_sitter::Node,
    source: &[u8],
    class_name: &str,
    field_types: &HashMap<String, String>,
    base_types: &[String],
) -> Option<String> {
    let text = node_text(receiver, source);
    match receiver.kind() {
        "identifier" => {
            let name = text.trim();
            match name {
                "this" => Some(class_name.to_string()),
                "base" => base_types.first().map(|bt| bt.split('<').next().unwrap_or(bt).to_string()),
                _ => {
                    if let Some(type_name) = field_types.get(name) {
                        Some(type_name.clone())
                    } else if name.chars().next().is_some_and(|c| c.is_uppercase()) {
                        Some(name.to_string())
                    } else {
                        None
                    }
                }
            }
        }
        "this_expression" => Some(class_name.to_string()),
        "base_expression" => base_types.first().map(|bt| bt.split('<').next().unwrap_or(bt).to_string()),
        _ => {
            let trimmed = text.trim();
            if trimmed == "this" {
                Some(class_name.to_string())
            } else if trimmed == "base" {
                base_types.first().map(|bt| bt.split('<').next().unwrap_or(bt).to_string())
            } else {
                None
            }
        }
    }
}

// ─── AST walking ────────────────────────────────────────────────────

/// Walk AST collecting definitions AND method/constructor nodes for call extraction.
fn walk_csharp_node_collecting<'a>(
    node: tree_sitter::Node<'a>,
    source: &[u8],
    file_id: u32,
    parent_name: Option<&str>,
    defs: &mut Vec<DefinitionEntry>,
    method_nodes: &mut Vec<(usize, tree_sitter::Node<'a>)>,
) {
    let kind = node.kind();

    match kind {
        "class_declaration" | "interface_declaration" | "struct_declaration"
        | "enum_declaration" | "record_declaration" => {
            if let Some(def) = extract_csharp_type_def(node, source, file_id, parent_name) {
                let name = def.name.clone();
                defs.push(def);
                for i in 0..node.child_count() {
                    let child = node.child(i).unwrap();
                    match child.kind() {
                        "declaration_list" | "enum_member_declaration_list" => {
                            walk_csharp_node_collecting(child, source, file_id, Some(&name), defs, method_nodes);
                        }
                        _ => {}
                    }
                }
                return;
            }
        }
        "method_declaration" => {
            if let Some(def) = extract_csharp_method_def(node, source, file_id, parent_name) {
                let idx = defs.len();
                defs.push(def);
                method_nodes.push((idx, node));
                return;
            }
        }
        "constructor_declaration" => {
            if let Some(def) = extract_csharp_constructor_def(node, source, file_id, parent_name) {
                let idx = defs.len();
                defs.push(def);
                method_nodes.push((idx, node));
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

    for i in 0..node.child_count() {
        walk_csharp_node_collecting(node.child(i).unwrap(), source, file_id, parent_name, defs, method_nodes);
    }
}

// ─── Definition extraction helpers ──────────────────────────────────

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
            if child.is_named() {
                base_types.push(node_text(child, source).to_string());
            }
        }
    }
    base_types
}

fn extract_csharp_type_def(
    node: tree_sitter::Node, source: &[u8], file_id: u32, parent_name: Option<&str>,
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
    let sig = build_type_signature(node, source);
    Some(DefinitionEntry {
        file_id, name, kind,
        line_start: node.start_position().row as u32 + 1,
        line_end: node.end_position().row as u32 + 1,
        parent: parent_name.map(|s| s.to_string()),
        signature: Some(sig), modifiers, attributes, base_types,
    })
}

fn build_type_signature(node: tree_sitter::Node, source: &[u8]) -> String {
    let start = node.start_byte();
    let mut end = node.end_byte();
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        if child.kind() == "declaration_list" || child.kind() == "{" {
            end = child.start_byte();
            break;
        }
    }
    let text = std::str::from_utf8(&source[start..end]).unwrap_or("");
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn extract_csharp_method_def(
    node: tree_sitter::Node, source: &[u8], file_id: u32, parent_name: Option<&str>,
) -> Option<DefinitionEntry> {
    let name_node = find_child_by_field(node, "name")?;
    let name = node_text(name_node, source).to_string();
    let modifiers = extract_modifiers(node, source);
    let attributes = extract_attributes(node, source);
    let sig = build_method_signature(node, source);
    Some(DefinitionEntry {
        file_id, name, kind: DefinitionKind::Method,
        line_start: node.start_position().row as u32 + 1,
        line_end: node.end_position().row as u32 + 1,
        parent: parent_name.map(|s| s.to_string()),
        signature: Some(sig), modifiers, attributes, base_types: Vec::new(),
    })
}

fn build_method_signature(node: tree_sitter::Node, source: &[u8]) -> String {
    let start = node.start_byte();
    let mut end = node.end_byte();
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
    node: tree_sitter::Node, source: &[u8], file_id: u32, parent_name: Option<&str>,
) -> Option<DefinitionEntry> {
    let name_node = find_child_by_field(node, "name")?;
    let name = node_text(name_node, source).to_string();
    let modifiers = extract_modifiers(node, source);
    let attributes = extract_attributes(node, source);
    let sig = build_method_signature(node, source);
    Some(DefinitionEntry {
        file_id, name, kind: DefinitionKind::Constructor,
        line_start: node.start_position().row as u32 + 1,
        line_end: node.end_position().row as u32 + 1,
        parent: parent_name.map(|s| s.to_string()),
        signature: Some(sig), modifiers, attributes, base_types: Vec::new(),
    })
}

fn extract_csharp_property_def(
    node: tree_sitter::Node, source: &[u8], file_id: u32, parent_name: Option<&str>,
) -> Option<DefinitionEntry> {
    let name_node = find_child_by_field(node, "name")?;
    let name = node_text(name_node, source).to_string();
    let modifiers = extract_modifiers(node, source);
    let attributes = extract_attributes(node, source);
    let type_node = find_child_by_field(node, "type");
    let type_str = type_node.map(|n| node_text(n, source)).unwrap_or("");
    let sig = format!("{} {}", type_str, name);
    Some(DefinitionEntry {
        file_id, name, kind: DefinitionKind::Property,
        line_start: node.start_position().row as u32 + 1,
        line_end: node.end_position().row as u32 + 1,
        parent: parent_name.map(|s| s.to_string()),
        signature: Some(sig.trim().to_string()), modifiers, attributes, base_types: Vec::new(),
    })
}

fn extract_csharp_field_defs(
    node: tree_sitter::Node, source: &[u8], file_id: u32, parent_name: Option<&str>,
    defs: &mut Vec<DefinitionEntry>,
) {
    let modifiers = extract_modifiers(node, source);
    let attributes = extract_attributes(node, source);
    if let Some(var_decl) = find_child_by_kind(node, "variable_declaration") {
        let type_node = find_child_by_field(var_decl, "type");
        let type_str = type_node.map(|n| node_text(n, source)).unwrap_or("");
        for i in 0..var_decl.child_count() {
            let child = var_decl.child(i).unwrap();
            if child.kind() == "variable_declarator"
                && let Some(name_node) = find_child_by_field(child, "name") {
                    let name = node_text(name_node, source).to_string();
                    let sig = format!("{} {}", type_str, name);
                    defs.push(DefinitionEntry {
                        file_id, name, kind: DefinitionKind::Field,
                        line_start: node.start_position().row as u32 + 1,
                        line_end: node.end_position().row as u32 + 1,
                        parent: parent_name.map(|s| s.to_string()),
                        signature: Some(sig.trim().to_string()),
                        modifiers: modifiers.clone(), attributes: attributes.clone(),
                        base_types: Vec::new(),
                    });
                }
        }
    }
}

fn extract_csharp_delegate_def(
    node: tree_sitter::Node, source: &[u8], file_id: u32, parent_name: Option<&str>,
) -> Option<DefinitionEntry> {
    let name_node = find_child_by_field(node, "name")?;
    let name = node_text(name_node, source).to_string();
    let modifiers = extract_modifiers(node, source);
    let attributes = extract_attributes(node, source);
    let sig_text = node_text(node, source);
    let sig = sig_text.split_whitespace().collect::<Vec<_>>().join(" ");
    Some(DefinitionEntry {
        file_id, name, kind: DefinitionKind::Delegate,
        line_start: node.start_position().row as u32 + 1,
        line_end: node.end_position().row as u32 + 1,
        parent: parent_name.map(|s| s.to_string()),
        signature: Some(sig), modifiers, attributes, base_types: Vec::new(),
    })
}

fn extract_csharp_event_def(
    node: tree_sitter::Node, source: &[u8], file_id: u32, parent_name: Option<&str>,
) -> Option<DefinitionEntry> {
    let name = if let Some(name_node) = find_child_by_field(node, "name") {
        node_text(name_node, source).to_string()
    } else {
        let var_decl = find_child_by_kind(node, "variable_declaration");
        if let Some(vd) = var_decl {
            let declarator = find_child_by_kind(vd, "variable_declarator");
            if let Some(d) = declarator {
                if let Some(n) = find_child_by_field(d, "name") {
                    node_text(n, source).to_string()
                } else { return None; }
            } else { return None; }
        } else { return None; }
    };
    let modifiers = extract_modifiers(node, source);
    let attributes = extract_attributes(node, source);
    Some(DefinitionEntry {
        file_id, name, kind: DefinitionKind::Event,
        line_start: node.start_position().row as u32 + 1,
        line_end: node.end_position().row as u32 + 1,
        parent: parent_name.map(|s| s.to_string()),
        signature: None, modifiers, attributes, base_types: Vec::new(),
    })
}

fn extract_csharp_enum_member(
    node: tree_sitter::Node, source: &[u8], file_id: u32, parent_name: Option<&str>,
) -> Option<DefinitionEntry> {
    let name_node = find_child_by_field(node, "name")?;
    let name = node_text(name_node, source).to_string();
    Some(DefinitionEntry {
        file_id, name, kind: DefinitionKind::EnumMember,
        line_start: node.start_position().row as u32 + 1,
        line_end: node.end_position().row as u32 + 1,
        parent: parent_name.map(|s| s.to_string()),
        signature: None, modifiers: Vec::new(), attributes: Vec::new(), base_types: Vec::new(),
    })
}