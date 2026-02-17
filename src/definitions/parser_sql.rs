//! SQL AST parser using tree-sitter (currently disabled — no compatible T-SQL grammar).

use super::types::*;

pub(crate) fn parse_sql_definitions(
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
            let text = node_text(node, source).to_uppercase();
            if text.contains("PROCEDURE") || text.contains("PROC") {
                if let Some(def) = extract_sql_named_def(node, source, file_id, DefinitionKind::StoredProcedure) {
                    defs.push(def);
                    return;
                }
            } else if let Some(def) = extract_sql_named_def(node, source, file_id, DefinitionKind::SqlFunction) {
                defs.push(def);
                return;
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
    let name = find_sql_object_name(node, source)?;

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
    if let Some(name_node) = node.child_by_field_name("name") {
        return Some(node_text(name_node, source).to_string());
    }

    let mut found_keyword = false;
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        let ck = child.kind();

        if ck == "keyword_create" || ck == "keyword_table" || ck == "keyword_procedure"
           || ck == "keyword_function" || ck == "keyword_view" || ck == "keyword_type"
           || ck == "keyword_index" || ck == "keyword_or" || ck == "keyword_replace"
           || ck == "keyword_alter" || ck == "keyword_unique" || ck == "keyword_clustered"
           || ck == "keyword_nonclustered" || ck == "keyword_if" || ck == "keyword_not"
           || ck == "keyword_exists" || ck == "CREATE" || ck == "TABLE" || ck == "PROCEDURE" {
            found_keyword = true;
            continue;
        }

        if found_keyword
            && (ck == "identifier" || ck == "dotted_name" || ck == "object_reference"
               || ck == "schema_qualified_name" || child.is_named()) {
                let text = node_text(child, source).to_string();
                if !text.is_empty() && !text.starts_with('(') && !text.starts_with("AS") {
                    return Some(text);
                }
            }
    }

    // Fallback: extract from raw text
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

fn node_text<'a>(node: tree_sitter::Node, source: &'a [u8]) -> &'a str {
    node.utf8_text(source).unwrap_or("")
}