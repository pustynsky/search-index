//! Persistence for DefinitionIndex: save/load/find on disk.

use std::path::PathBuf;

use crate::clean_path;

use super::types::DefinitionIndex;

fn def_index_path_for(dir: &str, exts: &str, index_base: &std::path::Path) -> PathBuf {
    let canonical = std::fs::canonicalize(dir).unwrap_or_else(|_| PathBuf::from(dir));
    let hash = search::stable_hash(&[
        canonical.to_string_lossy().as_bytes(),
        exts.as_bytes(),
        b"definitions", // distinguish from content index
    ]);
    index_base.join(format!("{:016x}.didx", hash))
}

pub fn save_definition_index(index: &DefinitionIndex, index_base: &std::path::Path) -> Result<(), crate::SearchError> {
    std::fs::create_dir_all(index_base)?;
    let exts_str = index.extensions.join(",");
    let path = def_index_path_for(&index.root, &exts_str, index_base);
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
pub fn load_definition_index(dir: &str, exts: &str, index_base: &std::path::Path) -> Option<DefinitionIndex> {
    let path = def_index_path_for(dir, exts, index_base);
    let data = std::fs::read(&path).ok()?;
    bincode::deserialize(&data).ok()
}

/// Try to find any definition index for a directory (any extension combo)
#[allow(dead_code)]
pub fn find_definition_index_for_dir(dir: &str, index_base: &std::path::Path) -> Option<DefinitionIndex> {
    let canonical = std::fs::canonicalize(dir).ok()?;
    let dir_str = clean_path(&canonical.to_string_lossy());
    let entries = std::fs::read_dir(index_base).ok()?;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("didx")
            && let Ok(data) = std::fs::read(&path)
                && let Ok(index) = bincode::deserialize::<DefinitionIndex>(&data) {
                    let idx_root = std::fs::canonicalize(&index.root)
                        .map(|p| clean_path(&p.to_string_lossy()))
                        .unwrap_or_else(|_| index.root.clone());
                    if idx_root.eq_ignore_ascii_case(&dir_str) {
                        return Some(index);
                    }
                }
    }
    None
}