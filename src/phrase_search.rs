use std::collections::HashMap;
use std::time::Instant;

use crate::ContentIndex;

pub(crate) struct PhraseCandidateScan {
    pub candidates: Vec<(u32, Vec<u32>)>,
    pub per_token: Vec<(String, usize, usize, f64)>,
    pub missing_tokens: Vec<String>,
    pub posting_scan_ms: f64,
    pub intersection_ms: f64,
}

pub(crate) fn collect_phrase_line_candidates(
    index: &ContentIndex,
    phrase_tokens: &[String],
    mut in_scope: impl FnMut(u32) -> bool,
) -> PhraseCandidateScan {
    let posting_scan_start = Instant::now();
    let mut per_token = Vec::with_capacity(phrase_tokens.len());
    let mut per_token_file_lines = Vec::with_capacity(phrase_tokens.len());

    for token in phrase_tokens {
        let token_start = Instant::now();
        let mut posting_count = 0usize;
        let mut matched_index_tokens = 0usize;
        let mut file_lines: HashMap<u32, Vec<u32>> = HashMap::new();
        for (indexed_token, postings) in &index.index {
            if !indexed_token.starts_with(token) {
                continue;
            }
            matched_index_tokens += 1;
            posting_count += postings.len();
            for posting in postings {
                if in_scope(posting.file_id) {
                    file_lines
                        .entry(posting.file_id)
                        .or_default()
                        .extend_from_slice(&posting.lines);
                }
            }
        }
        if matched_index_tokens == 0 {
            per_token.push((token.clone(), 0, 0, token_start.elapsed().as_secs_f64() * 1000.0));
            return PhraseCandidateScan {
                candidates: Vec::new(),
                per_token,
                missing_tokens: vec![token.clone()],
                posting_scan_ms: posting_scan_start.elapsed().as_secs_f64() * 1000.0,
                intersection_ms: 0.0,
            };
        }
        for lines in file_lines.values_mut() {
            lines.sort_unstable();
            lines.dedup();
        }
        per_token.push((
            token.clone(),
            posting_count,
            file_lines.len(),
            token_start.elapsed().as_secs_f64() * 1000.0,
        ));
        if file_lines.is_empty() {
            return PhraseCandidateScan {
                candidates: Vec::new(),
                per_token,
                missing_tokens: Vec::new(),
                posting_scan_ms: posting_scan_start.elapsed().as_secs_f64() * 1000.0,
                intersection_ms: 0.0,
            };
        }
        per_token_file_lines.push(file_lines);
    }

    let posting_scan_ms = posting_scan_start.elapsed().as_secs_f64() * 1000.0;
    let intersection_start = Instant::now();
    let candidates = if per_token_file_lines.is_empty() {
        Vec::new()
    } else {
        let smallest_index = per_token_file_lines
            .iter()
            .enumerate()
            .min_by_key(|(_, file_lines)| file_lines.len())
            .map(|(index, _)| index)
            .unwrap_or(0);
        let smallest = per_token_file_lines.swap_remove(smallest_index);
        let mut candidates = Vec::new();
        for (file_id, mut lines) in smallest {
            let mut matched = true;
            for other in &per_token_file_lines {
                let Some(other_lines) = other.get(&file_id) else {
                    matched = false;
                    break;
                };
                lines = intersect_sorted_unique(&lines, other_lines);
                if lines.is_empty() {
                    matched = false;
                    break;
                }
            }
            if matched {
                candidates.push((file_id, lines));
            }
        }
        candidates
    };

    PhraseCandidateScan {
        candidates,
        per_token,
        missing_tokens: Vec::new(),
        posting_scan_ms,
        intersection_ms: intersection_start.elapsed().as_secs_f64() * 1000.0,
    }
}

pub(crate) fn intersect_sorted_unique(left: &[u32], right: &[u32]) -> Vec<u32> {
    let mut intersection = Vec::with_capacity(left.len().min(right.len()));
    let (mut left_index, mut right_index) = (0, 0);
    while left_index < left.len() && right_index < right.len() {
        match left[left_index].cmp(&right[right_index]) {
            std::cmp::Ordering::Equal => {
                intersection.push(left[left_index]);
                left_index += 1;
                right_index += 1;
            }
            std::cmp::Ordering::Less => left_index += 1,
            std::cmp::Ordering::Greater => right_index += 1,
        }
    }
    intersection
}
