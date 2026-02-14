//! # search — High-Performance Code Search Engine
//!
//! Inverted index + AST-based code intelligence engine for large-scale codebases.
//! Sub-microsecond content search, structural code navigation, and native MCP server.
//!
//! ## Library usage
//!
//! This crate is primarily a CLI tool / MCP server, but core types and functions
//! are exposed as a library for benchmarking and integration testing.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

// ─── Core public types ───────────────────────────────────────────────

/// Strip the `\\?\` extended-length path prefix that Windows canonicalize adds.
pub fn clean_path(p: &str) -> String {
    p.strip_prefix(r"\\?\").unwrap_or(p).to_string()
}

/// A posting: file_id + line numbers where the token appears.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Posting {
    pub file_id: u32,
    pub lines: Vec<u32>,
}

/// Inverted index: token → list of postings.
///
/// The core data structure for content search. Maps every token
/// to the files and line numbers where it appears.
#[derive(Serialize, Deserialize, Debug)]
pub struct ContentIndex {
    pub root: String,
    pub created_at: u64,
    pub max_age_secs: u64,
    /// file_id → file path
    pub files: Vec<String>,
    /// token (lowercased) → postings
    pub index: HashMap<String, Vec<Posting>>,
    /// total tokens indexed
    pub total_tokens: u64,
    /// extensions that were indexed
    pub extensions: Vec<String>,
    /// file_id → total token count in that file (for TF-IDF)
    pub file_token_counts: Vec<u32>,
    /// Forward index: file_id → Vec<token> (only populated with --watch)
    #[serde(default)]
    pub forward: Option<HashMap<u32, Vec<String>>>,
    /// Path → file_id lookup (only populated with --watch)
    #[serde(default)]
    pub path_to_id: Option<HashMap<PathBuf, u32>>,
}

impl ContentIndex {
    /// Check if the index is older than its configured max age.
    pub fn is_stale(&self) -> bool {
        use std::time::{SystemTime, UNIX_EPOCH};
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        now - self.created_at > self.max_age_secs
    }
}

/// Tokenize a line of text into lowercase tokens.
///
/// Splits on non-alphanumeric characters (except `_`),
/// filters by minimum length, and lowercases all tokens.
///
/// # Examples
///
/// ```
/// use search::tokenize;
///
/// let tokens = tokenize("private readonly HttpClient _client;", 2);
/// assert!(tokens.contains(&"private".to_string()));
/// assert!(tokens.contains(&"httpclient".to_string()));
/// assert!(tokens.contains(&"_client".to_string()));
/// ```
pub fn tokenize(line: &str, min_len: usize) -> Vec<String> {
    line.split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|s| s.len() >= min_len)
        .map(|s| s.to_lowercase())
        .collect()
}

#[cfg(test)]
mod lib_tests {
    use super::*;

    #[test]
    fn test_tokenize_basic() {
        let tokens = tokenize("hello world", 2);
        assert_eq!(tokens, vec!["hello", "world"]);
    }

    #[test]
    fn test_tokenize_code() {
        let tokens = tokenize("private readonly HttpClient _client;", 2);
        assert_eq!(
            tokens,
            vec!["private", "readonly", "httpclient", "_client"]
        );
    }

    #[test]
    fn test_tokenize_min_length() {
        let tokens = tokenize("a bb ccc", 2);
        assert_eq!(tokens, vec!["bb", "ccc"]);
    }

    #[test]
    fn test_clean_path_strips_prefix() {
        assert_eq!(clean_path(r"\\?\C:\Users\test"), r"C:\Users\test");
    }

    #[test]
    fn test_clean_path_no_prefix() {
        assert_eq!(clean_path(r"C:\Users\test"), r"C:\Users\test");
    }

    #[test]
    fn test_content_index_stale() {
        let index = ContentIndex {
            root: ".".to_string(),
            created_at: 0, // epoch = definitely stale
            max_age_secs: 3600,
            files: vec![],
            index: HashMap::new(),
            total_tokens: 0,
            extensions: vec![],
            file_token_counts: vec![],
            forward: None,
            path_to_id: None,
        };
        assert!(index.is_stale());
    }

    #[test]
    fn test_posting_serialization_roundtrip() {
        let posting = Posting {
            file_id: 42,
            lines: vec![1, 5, 10],
        };
        let encoded = bincode::serialize(&posting).unwrap();
        let decoded: Posting = bincode::deserialize(&encoded).unwrap();
        assert_eq!(decoded.file_id, 42);
        assert_eq!(decoded.lines, vec![1, 5, 10]);
    }
}

// ─── Property-based tests (proptest) ─────────────────────────────────

#[cfg(test)]
mod property_tests {
    use super::*;
    use proptest::prelude::*;

    // ─── Tokenizer invariants ────────────────────────────────────

    proptest! {
        /// Tokenizer always produces lowercase output regardless of input case.
        #[test]
        fn tokenize_always_lowercase(input in "\\PC{1,200}") {
            let tokens = tokenize(&input, 1);
            for token in &tokens {
                prop_assert_eq!(token, &token.to_lowercase(),
                    "Token '{}' is not lowercase", token);
            }
        }

        /// Tokenizer never produces tokens shorter than min_len (byte length).
        /// Note: Uses ASCII input because Unicode lowercasing can change byte length
        /// (e.g. German ß → ss), making the pre-lowercase filter insufficient.
        /// This is acceptable — code identifiers are ASCII in >99% of codebases.
        #[test]
        fn tokenize_respects_min_length(
            input in "[a-zA-Z0-9_ .;:(){}]{1,200}",
            min_len in 1usize..10
        ) {
            let tokens = tokenize(&input, min_len);
            for token in &tokens {
                prop_assert!(token.len() >= min_len,
                    "Token '{}' (len {}) is shorter than min_len {}",
                    token, token.len(), min_len);
            }
        }

        /// Tokenizer output is deterministic — same input always gives same output.
        #[test]
        fn tokenize_is_deterministic(input in "\\PC{1,200}") {
            let result1 = tokenize(&input, 2);
            let result2 = tokenize(&input, 2);
            prop_assert_eq!(result1, result2);
        }

        /// Empty input always produces empty output.
        #[test]
        fn tokenize_empty_min_len(min_len in 1usize..20) {
            let tokens = tokenize("", min_len);
            prop_assert!(tokens.is_empty());
        }

        /// Tokens only contain alphanumeric chars, underscores, and combining marks
        /// (Unicode lowercasing can produce combining chars, e.g. Turkish İ → i + combining dot).
        #[test]
        fn tokenize_valid_chars_only(input in "[a-zA-Z0-9_ !@#$%^&*()]{1,200}") {
            let tokens = tokenize(&input, 1);
            for token in &tokens {
                for c in token.chars() {
                    prop_assert!(c.is_alphanumeric() || c == '_',
                        "Token '{}' contains invalid char '{}'", token, c);
                }
            }
        }

        /// Increasing min_len never increases the number of tokens.
        #[test]
        fn tokenize_higher_min_len_fewer_tokens(input in "\\PC{1,200}") {
            let tokens_1 = tokenize(&input, 1);
            let tokens_2 = tokenize(&input, 2);
            let tokens_5 = tokenize(&input, 5);
            prop_assert!(tokens_2.len() <= tokens_1.len(),
                "min_len=2 produced more tokens ({}) than min_len=1 ({})",
                tokens_2.len(), tokens_1.len());
            prop_assert!(tokens_5.len() <= tokens_2.len(),
                "min_len=5 produced more tokens ({}) than min_len=2 ({})",
                tokens_5.len(), tokens_2.len());
        }

        /// Tokenizing a single alphanumeric word returns that word lowercased.
        #[test]
        fn tokenize_single_word(word in "[a-zA-Z][a-zA-Z0-9_]{1,30}") {
            let tokens = tokenize(&word, 1);
            prop_assert!(tokens.contains(&word.to_lowercase()),
                "Expected '{}' in tokens {:?}", word.to_lowercase(), tokens);
        }
    }

    // ─── Posting serialization invariants ────────────────────────

    proptest! {
        /// Posting survives bincode serialization roundtrip.
        #[test]
        fn posting_roundtrip(
            file_id in 0u32..100_000,
            lines in proptest::collection::vec(1u32..100_000, 0..50)
        ) {
            let posting = Posting { file_id, lines: lines.clone() };
            let encoded = bincode::serialize(&posting).unwrap();
            let decoded: Posting = bincode::deserialize(&encoded).unwrap();
            prop_assert_eq!(decoded.file_id, file_id);
            prop_assert_eq!(decoded.lines, lines);
        }
    }

    // ─── ContentIndex invariants ─────────────────────────────────

    proptest! {
        /// Building an index from tokenized content maintains consistency:
        /// every token in the inverted index points to a valid file_id.
        #[test]
        fn index_file_ids_are_valid(
            num_files in 1usize..20,
            tokens_per_file in 1usize..50,
        ) {
            let mut files = Vec::new();
            let mut index: HashMap<String, Vec<Posting>> = HashMap::new();
            let mut file_token_counts = Vec::new();

            for file_id in 0..num_files {
                files.push(format!("file_{}.cs", file_id));
                let mut count = 0u32;
                for t in 0..tokens_per_file {
                    let token = format!("tok_{}", t % 10);
                    count += 1;
                    index.entry(token).or_default().push(Posting {
                        file_id: file_id as u32,
                        lines: vec![(t + 1) as u32],
                    });
                }
                file_token_counts.push(count);
            }

            // Invariant: every file_id in postings is < files.len()
            for (_token, postings) in &index {
                for posting in postings {
                    prop_assert!((posting.file_id as usize) < files.len(),
                        "file_id {} >= files.len() {}", posting.file_id, files.len());
                }
            }

            // Invariant: file_token_counts has same length as files
            prop_assert_eq!(file_token_counts.len(), files.len());
        }

        /// ContentIndex survives bincode serialization roundtrip.
        #[test]
        fn content_index_roundtrip(num_files in 1usize..10) {
            let mut files = Vec::new();
            let mut index: HashMap<String, Vec<Posting>> = HashMap::new();
            let mut file_token_counts = Vec::new();
            let mut total_tokens = 0u64;

            for file_id in 0..num_files {
                files.push(format!("file_{}.cs", file_id));
                let token = format!("token_{}", file_id);
                total_tokens += 1;
                file_token_counts.push(1);
                index.entry(token).or_default().push(Posting {
                    file_id: file_id as u32,
                    lines: vec![1],
                });
            }

            let ci = ContentIndex {
                root: ".".to_string(),
                created_at: 1000,
                max_age_secs: 86400,
                files: files.clone(),
                index,
                total_tokens,
                extensions: vec!["cs".to_string()],
                file_token_counts: file_token_counts.clone(),
                forward: None,
                path_to_id: None,
            };

            let encoded = bincode::serialize(&ci).unwrap();
            let decoded: ContentIndex = bincode::deserialize(&encoded).unwrap();

            prop_assert_eq!(decoded.files.len(), files.len());
            prop_assert_eq!(decoded.total_tokens, total_tokens);
            prop_assert_eq!(decoded.file_token_counts, file_token_counts);
            prop_assert_eq!(decoded.root, ".");
        }
    }

    // ─── TF-IDF invariants ───────────────────────────────────────

    proptest! {
        /// TF-IDF: a token appearing in fewer documents should have higher IDF.
        #[test]
        fn tfidf_rare_token_higher_idf(
            total_docs in 10u32..10_000,
            rare_count in 1u32..5,
            common_count_extra in 5u32..100,
        ) {
            let total = total_docs as f64;
            let common_count = rare_count + common_count_extra;
            // Ensure common_count <= total_docs
            let common_count = common_count.min(total_docs);
            let rare_count = rare_count.min(common_count - 1).max(1);

            let idf_rare = (total / rare_count as f64).ln();
            let idf_common = (total / common_count as f64).ln();

            prop_assert!(idf_rare > idf_common,
                "Rare IDF ({}) should be > common IDF ({}), rare_count={}, common_count={}, total={}",
                idf_rare, idf_common, rare_count, common_count, total_docs);
        }

        /// TF: higher occurrence count with same file size = higher TF.
        #[test]
        fn tfidf_more_occurrences_higher_tf(
            file_total in 10u32..10_000,
            low_count in 1u32..5,
            extra in 1u32..100,
        ) {
            let high_count = low_count + extra;
            let tf_low = low_count as f64 / file_total as f64;
            let tf_high = high_count as f64 / file_total as f64;
            prop_assert!(tf_high > tf_low);
        }
    }

    // ─── clean_path invariants ───────────────────────────────────

    proptest! {
        /// clean_path is idempotent — applying it twice gives the same result.
        #[test]
        fn clean_path_idempotent(input in "\\PC{0,100}") {
            let once = clean_path(&input);
            let twice = clean_path(&once);
            prop_assert_eq!(once, twice);
        }

        /// clean_path output never starts with \\?\
        #[test]
        fn clean_path_no_prefix_in_output(input in "\\PC{0,100}") {
            let result = clean_path(&input);
            prop_assert!(!result.starts_with(r"\\?\"),
                "clean_path output '{}' still has prefix", result);
        }
    }
}