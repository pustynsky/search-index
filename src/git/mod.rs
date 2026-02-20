//! Git history query module — calls `git` CLI for optimal performance.
//!
//! Uses `git log` CLI with commit-graph and bloom filter optimizations for
//! path-limited queries. On-demand fallback when in-memory cache is not available.
//! See `cache.rs` for the pre-built in-memory cache path (sub-millisecond queries).

use std::collections::HashMap;
use std::process::Command;

// ─── Types ──────────────────────────────────────────────────────────

/// Information about a single commit that touched a file.
#[derive(Clone, Debug)]
pub struct CommitInfo {
    pub hash: String,
    pub date: String,
    pub author_name: String,
    pub author_email: String,
    pub message: String,
    pub patch: Option<String>,
}

/// Aggregated author statistics for a file.
#[derive(Clone, Debug)]
pub struct AuthorStats {
    pub name: String,
    pub email: String,
    pub commit_count: usize,
    pub first_change: String,
    pub last_change: String,
}

/// Date range filter for git queries.
#[derive(Clone, Debug)]
pub struct DateFilter {
    /// Start date string (YYYY-MM-DD), inclusive
    pub from_date: Option<String>,
    /// End date string (YYYY-MM-DD), inclusive (converted to next day for git --before)
    pub to_date: Option<String>,
}

// ─── Date helpers ───────────────────────────────────────────────────

/// Validate a YYYY-MM-DD date string. Returns Ok(()) or Err with message.
pub fn validate_date(s: &str) -> Result<(), String> {
    // Simple validation: must be YYYY-MM-DD format
    if s.len() != 10 {
        return Err(format!("Invalid date '{}': expected YYYY-MM-DD format", s));
    }
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 3 {
        return Err(format!("Invalid date '{}': expected YYYY-MM-DD format", s));
    }
    let year: u32 = parts[0].parse().map_err(|_| format!("Invalid year in '{}'", s))?;
    let month: u32 = parts[1].parse().map_err(|_| format!("Invalid month in '{}'", s))?;
    let day: u32 = parts[2].parse().map_err(|_| format!("Invalid day in '{}'", s))?;

    if year < 1970 || year > 2100 {
        return Err(format!("Year {} out of range (1970-2100)", year));
    }
    if month < 1 || month > 12 {
        return Err(format!("Month {} out of range (1-12)", month));
    }
    if day < 1 || day > 31 {
        return Err(format!("Day {} out of range (1-31)", day));
    }

    Ok(())
}

/// Increment a YYYY-MM-DD date by one day for --before filter.
/// Simple implementation that handles month/year boundaries.
fn next_day(date: &str) -> String {
    let parts: Vec<u32> = date.split('-').filter_map(|p| p.parse().ok()).collect();
    if parts.len() != 3 {
        return format!("{}T23:59:59", date); // fallback
    }
    let (year, month, day) = (parts[0], parts[1], parts[2]);

    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) { 29 } else { 28 },
        _ => 31,
    };

    if day < days_in_month {
        format!("{:04}-{:02}-{:02}", year, month, day + 1)
    } else if month < 12 {
        format!("{:04}-{:02}-01", year, month + 1)
    } else {
        format!("{:04}-01-01", year + 1)
    }
}

/// Build a DateFilter from optional from/to/date parameters.
///
/// If `date` is provided, it overrides `from` and `to` (single-day filter).
pub fn parse_date_filter(
    from: Option<&str>,
    to: Option<&str>,
    date: Option<&str>,
) -> Result<DateFilter, String> {
    if let Some(d) = date {
        validate_date(d)?;
        Ok(DateFilter {
            from_date: Some(d.to_string()),
            to_date: Some(d.to_string()),
        })
    } else {
        if let Some(f) = from {
            validate_date(f)?;
        }
        if let Some(t) = to {
            validate_date(t)?;
        }
        Ok(DateFilter {
            from_date: from.map(|s| s.to_string()),
            to_date: to.map(|s| s.to_string()),
        })
    }
}

// ─── Git CLI helpers ────────────────────────────────────────────────

/// Separator used in git log --format to split fields.
/// Using a rare Unicode character to avoid collision with commit messages.
const FIELD_SEP: &str = "␞";
/// Separator between records in git log output.
const RECORD_SEP: &str = "␟";

/// Build common git log arguments for date filtering.
///
/// Appends `T00:00:00Z` to force UTC interpretation, matching the cache path
/// which uses UTC timestamps. Without this, git interprets bare YYYY-MM-DD
/// dates in the local timezone, causing mismatches on non-UTC systems.
fn add_date_args(cmd: &mut Command, filter: &DateFilter) {
    if let Some(ref from) = filter.from_date {
        cmd.arg(format!("--after={}T00:00:00Z", from));
    }
    if let Some(ref to) = filter.to_date {
        // git --before is exclusive, so we need the next day for inclusive behavior
        let next = next_day(to);
        cmd.arg(format!("--before={}T00:00:00Z", next));
    }
}

/// Run a git command and return stdout as String.
fn run_git(cmd: &mut Command) -> Result<String, String> {
    let output = cmd
        .output()
        .map_err(|e| format!("Failed to execute git: {}. Is git installed and in PATH?", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git command failed: {}", stderr.trim()));
    }

    String::from_utf8(output.stdout)
        .map_err(|e| format!("git output is not valid UTF-8: {}", e))
}

/// Parse a git log record (using FIELD_SEP-separated fields) into CommitInfo.
fn parse_commit_record(record: &str) -> Option<CommitInfo> {
    let fields: Vec<&str> = record.split(FIELD_SEP).collect();
    if fields.len() < 5 {
        return None;
    }

    Some(CommitInfo {
        hash: fields[0].trim().to_string(),
        date: fields[1].trim().to_string(),
        author_name: fields[2].trim().to_string(),
        author_email: fields[3].trim().to_string(),
        message: fields[4..].join(FIELD_SEP).trim().to_string(),
        patch: None,
    })
}

// ─── Core query functions ───────────────────────────────────────────

/// Maximum number of patch lines per commit to prevent context overflow.
const MAX_PATCH_LINES: usize = 200;

/// Get commit history for a single file.
///
/// If `include_diff` is true, each commit includes the patch text.
/// `max_results` limits the number of commits returned (0 = unlimited).
///
/// Returns `(commits, total_count)` where total_count may exceed commits.len()
/// when max_results limits the output.
pub fn file_history(
    repo_path: &str,
    file: &str,
    filter: &DateFilter,
    include_diff: bool,
    max_results: usize,
) -> Result<(Vec<CommitInfo>, usize), String> {
    // First, get the commit list (always fast with git CLI + commit-graph)
    let format = format!("{}%H{}%ai{}%an{}%ae{}%s{}", RECORD_SEP, FIELD_SEP, FIELD_SEP, FIELD_SEP, FIELD_SEP, FIELD_SEP);

    let mut cmd = Command::new("git");
    cmd.current_dir(repo_path)
        .arg("log")
        .arg(format!("--format={}", format))
        .arg("--follow"); // follow renames

    add_date_args(&mut cmd, filter);

    cmd.arg("--").arg(file);

    let output = run_git(&mut cmd)?;

    let mut commits: Vec<CommitInfo> = output
        .split(RECORD_SEP)
        .filter(|s| !s.trim().is_empty())
        .filter_map(parse_commit_record)
        .collect();

    let total_count = commits.len();

    // Apply max_results
    if max_results > 0 && commits.len() > max_results {
        commits.truncate(max_results);
    }

    // If diff requested, get patch for each commit
    if include_diff {
        for commit in &mut commits {
            let patch = get_commit_diff(repo_path, &commit.hash, file)?;
            commit.patch = Some(patch);
        }
    }

    Ok((commits, total_count))
}

/// Get the diff/patch for a specific commit and file.
fn get_commit_diff(repo_path: &str, hash: &str, file: &str) -> Result<String, String> {
    let mut cmd = Command::new("git");
    cmd.current_dir(repo_path)
        .arg("diff")
        .arg(format!("{}^..{}", hash, hash))
        .arg("--")
        .arg(file);

    let output = run_git(&mut cmd)?;

    // Truncate to MAX_PATCH_LINES
    let lines: Vec<&str> = output.lines().collect();
    if lines.len() > MAX_PATCH_LINES {
        let truncated: String = lines[..MAX_PATCH_LINES].join("\n");
        Ok(format!("{}\n... (truncated at {} lines)", truncated, MAX_PATCH_LINES))
    } else {
        Ok(output)
    }
}

/// Get top authors for a file, ranked by commit count.
///
/// Returns `(authors, total_commits, total_authors)`.
pub fn top_authors(
    repo_path: &str,
    file: &str,
    filter: &DateFilter,
    top: usize,
) -> Result<(Vec<AuthorStats>, usize, usize), String> {
    // Use git shortlog for author aggregation (much faster than manual counting)
    // But git shortlog doesn't give us first/last dates, so we use git log
    let format = format!("{}%H{}%ai{}%an{}%ae{}%s{}", RECORD_SEP, FIELD_SEP, FIELD_SEP, FIELD_SEP, FIELD_SEP, FIELD_SEP);

    let mut cmd = Command::new("git");
    cmd.current_dir(repo_path)
        .arg("log")
        .arg(format!("--format={}", format))
        .arg("--follow");

    add_date_args(&mut cmd, filter);

    cmd.arg("--").arg(file);

    let output = run_git(&mut cmd)?;

    let commits: Vec<CommitInfo> = output
        .split(RECORD_SEP)
        .filter(|s| !s.trim().is_empty())
        .filter_map(parse_commit_record)
        .collect();

    // Aggregate by author
    #[derive(Default)]
    struct InternalStats {
        count: usize,
        name: String,
        email: String,
        first_date: Option<String>,
        last_date: Option<String>,
    }

    let mut author_map: HashMap<String, InternalStats> = HashMap::new();

    for commit in &commits {
        let key = format!("{} <{}>", commit.author_name, commit.author_email);
        let stats = author_map.entry(key).or_insert_with(|| InternalStats {
            name: commit.author_name.clone(),
            email: commit.author_email.clone(),
            ..Default::default()
        });
        stats.count += 1;
        // Commits come in reverse chronological order
        if stats.last_date.is_none() {
            stats.last_date = Some(commit.date.clone());
        }
        stats.first_date = Some(commit.date.clone()); // keeps getting overwritten to oldest
    }

    let total_commits: usize = author_map.values().map(|s| s.count).sum();
    let total_authors = author_map.len();

    let mut ranked: Vec<_> = author_map.into_values().collect();
    ranked.sort_by(|a, b| b.count.cmp(&a.count));
    ranked.truncate(top);

    let authors: Vec<AuthorStats> = ranked
        .into_iter()
        .map(|s| AuthorStats {
            name: s.name,
            email: s.email,
            commit_count: s.count,
            first_change: s.first_date.unwrap_or_default(),
            last_change: s.last_date.unwrap_or_default(),
        })
        .collect();

    Ok((authors, total_commits, total_authors))
}

/// Get activity across ALL files in a repo for a date range.
///
/// Returns `(file_map, commits_processed)` where file_map maps
/// file paths to their commits.
pub fn repo_activity(
    repo_path: &str,
    filter: &DateFilter,
) -> Result<(HashMap<String, Vec<CommitInfo>>, u64), String> {
    // Use git log with --name-only to get changed files per commit
    let format = format!("{}%H{}%ai{}%an{}%ae{}%s{}", RECORD_SEP, FIELD_SEP, FIELD_SEP, FIELD_SEP, FIELD_SEP, FIELD_SEP);

    let mut cmd = Command::new("git");
    cmd.current_dir(repo_path)
        .arg("log")
        .arg(format!("--format={}", format))
        .arg("--name-only");

    add_date_args(&mut cmd, filter);

    let output = run_git(&mut cmd)?;

    let mut file_history: HashMap<String, Vec<CommitInfo>> = HashMap::new();
    let mut commits_processed = 0u64;

    // Parse output: each record starts with RECORD_SEP, followed by commit info,
    // then blank line, then file names (one per line)
    for record in output.split(RECORD_SEP) {
        let record = record.trim();
        if record.is_empty() {
            continue;
        }

        // Split at blank line: first part is commit info, rest is file list
        let parts: Vec<&str> = record.splitn(2, "\n\n").collect();

        let commit_info_str = parts[0];
        let file_list_str = if parts.len() > 1 { parts[1] } else { "" };

        if let Some(info) = parse_commit_record(commit_info_str) {
            commits_processed += 1;

            for file_line in file_list_str.lines() {
                let file_path = file_line.trim();
                if !file_path.is_empty() {
                    file_history
                        .entry(file_path.to_string())
                        .or_default()
                        .push(info.clone());
                }
            }
        }
    }

    Ok((file_history, commits_processed))
}

pub mod cache;

// ─── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "git_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "cache_tests.rs"]
mod cache_tests;