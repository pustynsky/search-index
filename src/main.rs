//! High-performance code search engine with inverted indexing and AST-based code intelligence.
//!
//! Binary crate entry point. All CLI logic is in the `cli` module.

// Re-export core types from library crate
pub use search::{clean_path, tokenize, ContentIndex, FileEntry, FileIndex, Posting, TrigramIndex};

mod cli;
mod definitions;
mod error;
mod index;
mod mcp;
mod tips;

pub use error::SearchError;
pub use index::{
    build_content_index, build_index, cleanup_orphaned_indexes, content_index_path_for,
    find_content_index_for_dir, index_dir, index_path_for, load_content_index, load_index,
    save_content_index, save_index,
};

// Re-export CLI types used by other modules
pub use cli::args::{IndexArgs, ContentIndexArgs, ServeArgs};
pub use cli::cmd_info_json;

fn main() {
    cli::run();
}

// ─── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
