use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::grep::{with_test_lexical_scoring_model, LexicalScoringModel};
use super::handlers_test_utils::HandlerContextBuilder;
use super::{dispatch_tool, HandlerContext};

const REPORT_SCHEMA_VERSION: u32 = 3;
const CANDIDATE_REPORT_SCHEMA_VERSION: u32 = 1;
const PRODUCTION_RELEVANCE_MODEL: &str = "tfidf-file-stem-v1";
const HYBRID_RELEVANCE_MODEL: &str = "hybrid-bm25-or-tfidf-and-v1";
const EXPECTED_QUERY_COUNT: usize = 40;
const EXPECTED_TFIDF_QUERY_COUNT: usize = 35;
const EXPECTED_CLASS_COUNT: usize = 8;
const EXPECTED_QUERIES_PER_CLASS: usize = 5;
const QUALITY_CUTOFF: usize = 10;
const RECALL_CUTOFF: usize = 50;
const USEFUL_GRADE: u8 = 2;

#[derive(Clone, Copy, Debug, PartialEq)]
enum RelevanceScoringPolicy {
    Fixed(LexicalScoringModel),
    HybridBm25OrTfidfAnd,
}

impl From<LexicalScoringModel> for RelevanceScoringPolicy {
    fn from(model: LexicalScoringModel) -> Self {
        Self::Fixed(model)
    }
}

impl RelevanceScoringPolicy {
    fn model_for_request(self, request: &Value) -> LexicalScoringModel {
        match self {
            Self::Fixed(model) => model,
            Self::HybridBm25OrTfidfAnd => {
                let request = request.as_object().expect("validated request object");
                let uses_and = request.get("mode").and_then(Value::as_str) == Some("and");
                let uses_unvalidated_mode = ["phrase", "lineRegex", "regex"].iter()
                    .any(|field| request.get(*field).and_then(Value::as_bool) == Some(true));
                if uses_and || uses_unvalidated_mode {
                    LexicalScoringModel::TfIdfFileStem
                } else {
                    LexicalScoringModel::Bm25 { k1: 2.0, b: 0.5 }
                }
            }
        }
    }

    fn is_production(self) -> bool {
        self == Self::Fixed(LexicalScoringModel::TfIdfFileStem)
    }
}

#[derive(Clone, Debug, PartialEq)]
struct RelevanceScorerConfig {
    scoring_policy: RelevanceScoringPolicy,
    default_label: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RelevanceSpec {
    schema_version: u32,
    corpus_version: String,
    extensions: Vec<String>,
    #[serde(default)]
    global_negatives: Vec<ExplicitNegative>,
    queries: Vec<RelevanceQuery>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RelevanceQuery {
    id: String,
    query_class: String,
    intent: String,
    request: Value,
    judgments: Vec<Judgment>,
    #[serde(default)]
    negatives: Vec<ExplicitNegative>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Judgment {
    path: String,
    grade: u8,
    reason: String,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExplicitNegative {
    path: String,
    reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MetricSet {
    query_count: usize,
    ndcg_at_10: f64,
    mrr_at_10: f64,
    recall_at_50: f64,
    success_at_1: f64,
    explicit_negative_hits_at_10: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct QueryQuality {
    id: String,
    query_class: String,
    search_mode: String,
    ndcg_at_10: f64,
    mrr_at_10: f64,
    recall_at_50: f64,
    success_at_1: bool,
    explicit_negative_hits_at_10: usize,
    top_paths: Vec<String>,
    missing_judgments: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct QualityReport {
    schema_version: u32,
    corpus_version: String,
    model: String,
    metrics: MetricSet,
    scored_metrics: MetricSet,
    per_class: BTreeMap<String, MetricSet>,
    query_digest: String,
    corpus_digest: String,
    queries: Vec<QueryQuality>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AggregateBaseline {
    schema_version: u32,
    corpus_version: String,
    model: String,
    metrics: MetricSet,
    scored_metrics: MetricSet,
    per_class: BTreeMap<String, MetricSet>,
    query_digest: String,
    corpus_digest: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LatencySummary {
    samples: usize,
    p50_micros: u128,
    p95_micros: u128,
    max_micros: u128,
    slowest_queries: Vec<QueryLatency>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct QueryLatency {
    id: String,
    query_class: String,
    micros: u128,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OfflineReport {
    quality: QualityReport,
    latency: LatencySummary,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CandidateSpec {
    schema_version: u32,
    corpus_version: String,
    extensions: Vec<String>,
    queries: Vec<CandidateQuery>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CandidateQuery {
    id: String,
    query_class: String,
    intent: String,
    request: Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CandidateReport {
    schema_version: u32,
    corpus_version: String,
    model: String,
    candidate_digest: String,
    corpus_digest: String,
    queries: Vec<QueryCandidates>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct QueryCandidates {
    id: String,
    query_class: String,
    intent: String,
    request: Value,
    search_mode: String,
    candidates: Vec<String>,
}

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("benches/fixtures/relevance")
}

fn push_digest_field(buffer: &mut Vec<u8>, value: &[u8]) {
    buffer.extend_from_slice(&(value.len() as u64).to_le_bytes());
    buffer.extend_from_slice(value);
}

fn collect_corpus_files(root: &Path, directory: &Path, files: &mut Vec<(String, PathBuf)>) {
    let entries = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", directory.display()));
    for entry in entries {
        let entry = entry.expect("relevance corpus entry should be readable");
        let file_type = entry.file_type().expect("relevance corpus file type should be readable");
        let path = entry.path();
        if file_type.is_symlink() {
            panic!("relevance corpus symlinks are not supported: {}", path.display());
        } else if file_type.is_dir() {
            collect_corpus_files(root, &path, files);
        } else if file_type.is_file() {
            let relative = path.strip_prefix(root)
                .expect("relevance corpus file should be below its root");
            files.push((crate::clean_path(&relative.to_string_lossy()), path));
        }
    }
}

fn corpus_digest(corpus_root: &Path, extensions: &[String]) -> String {
    let mut files = Vec::new();
    collect_corpus_files(corpus_root, corpus_root, &mut files);
    files.sort_by(|left, right| left.0.cmp(&right.0));

    let mut digest_input = Vec::new();
    push_digest_field(&mut digest_input, b"xray-relevance-corpus-v1");
    push_digest_field(&mut digest_input, b"extensions");
    for extension in extensions {
        push_digest_field(&mut digest_input, extension.as_bytes());
    }
    push_digest_field(&mut digest_input, b"files");

    for (relative_path, path) in files {
        push_digest_field(&mut digest_input, relative_path.as_bytes());
        let content = fs::read(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        let content_length = (content.len() as u64).to_le_bytes();
        let content_hash = code_xray::stable_hash(&[&content_length, &content]).to_le_bytes();
        push_digest_field(&mut digest_input, &content_hash);
    }
    format!("{:016x}", code_xray::stable_hash(&[&digest_input]))
}

fn uses_tfidf_search_mode(search_mode: &str) -> bool {
    matches!(
        search_mode,
        "or" | "and" | "regex" | "substring-or" | "substring-and"
    )
}

fn physical_target_path(path: &Path) -> Option<PathBuf> {
    let mut missing_suffix = Vec::new();
    let mut current = path;
    loop {
        if let Ok(mut resolved) = fs::canonicalize(current) {
            for component in missing_suffix.iter().rev() {
                resolved.push(component);
            }
            return Some(resolved);
        }
        missing_suffix.push(current.file_name()?.to_os_string());
        current = current.parent()?;
    }
}

fn validate_report_output_path(path: &Path) -> Result<PathBuf, String> {
    let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        repository_root.join(path)
    };
    let target_root = repository_root.join("target");
    let absolute_physical = physical_target_path(&absolute)
        .ok_or_else(|| format!("cannot resolve output path: {}", absolute.display()))?;
    let repository_physical = physical_target_path(&repository_root)
        .ok_or_else(|| format!("cannot resolve repository root: {}", repository_root.display()))?;
    let target_physical = physical_target_path(&target_root)
        .ok_or_else(|| format!("cannot resolve target root: {}", target_root.display()))?;
    let absolute_text = absolute_physical.to_string_lossy();
    let repository_text = repository_physical.to_string_lossy();
    let target_text = target_physical.to_string_lossy();
    if code_xray::is_path_within(&absolute_text, &repository_text)
        && !code_xray::is_path_within(&absolute_text, &target_text)
    {
        return Err(format!(
            "relevance reports inside the repository must stay under {} (input {}, physical {})",
            target_root.display(),
            absolute.display(),
            absolute_physical.display()
        ));
    }
    Ok(absolute)
}

fn load_spec_from(path: &Path) -> RelevanceSpec {
    let content = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    serde_json::from_str(&content)
        .unwrap_or_else(|error| panic!("failed to parse {}: {error}", path.display()))
}

fn load_spec() -> RelevanceSpec {
    load_spec_from(&fixture_root().join("queries.json"))
}

fn load_baseline() -> AggregateBaseline {
    let path = fixture_root().join("baseline-tfidf.json");
    let content = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    serde_json::from_str(&content)
        .unwrap_or_else(|error| panic!("failed to parse {}: {error}", path.display()))
}

fn validate_relative_path(path: &str, context: &str) {
    let parsed = Path::new(path);
    assert!(!parsed.is_absolute(), "{context} uses an absolute path: {path}");
    assert!(!path.contains(':') && !path.contains('\\'),
        "{context} must use a portable relative path: {path}");
    assert!(parsed.components().all(|component| {
        matches!(component, std::path::Component::Normal(_))
    }), "{context} contains a non-normal path component: {path}");
}


fn validate_query_request(id: &str, request: &Value) {
    let request = request.as_object()
        .unwrap_or_else(|| panic!("{id} request must be an object"));
    assert_ne!(request.get("countOnly").and_then(Value::as_bool), Some(true),
        "{id} cannot use countOnly");
    assert_ne!(request.get("filesOnly").and_then(Value::as_bool), Some(true),
        "{id} cannot use filesOnly");
    assert_ne!(request.get("invert").and_then(Value::as_bool), Some(true),
        "{id} cannot use invert");
    assert!(request.get("maxResults").is_none(),
        "{id} cannot set maxResults; the evaluator owns the cutoff");
    assert!(request.get("showLines").is_none(),
        "{id} cannot set showLines; the evaluator disables source payloads");
    let terms = request.get("terms").and_then(Value::as_array)
        .unwrap_or_else(|| panic!("{id} request must contain terms[]"));
    assert!(!terms.is_empty(), "{id} has no terms");
    assert!(terms.iter().all(Value::is_string), "{id} has a non-string term");
}

fn validate_model_sensitive_queries<'a, P>(
    scoring_policies: impl IntoIterator<Item = P>,
    queries: impl IntoIterator<Item = (&'a str, &'a Value)>,
) where
    P: Into<RelevanceScoringPolicy>,
{
    let has_experimental_policy = scoring_policies.into_iter()
        .map(Into::into)
        .any(|policy| !policy.is_production());
    if !has_experimental_policy {
        return;
    }
    for (id, request) in queries {
        let request = request.as_object().expect("validated request object");
        assert_ne!(request.get("phrase").and_then(Value::as_bool), Some(true),
            "{id} cannot use phrase because it bypasses lexical scoring");
        assert_ne!(request.get("lineRegex").and_then(Value::as_bool), Some(true),
            "{id} cannot use lineRegex because it bypasses lexical scoring");
    }
}

fn validate_model_sensitive_response(
    id: &str,
    scoring_policy: impl Into<RelevanceScoringPolicy>,
    output: &Value,
) {
    if scoring_policy.into().is_production() {
        return;
    }
    assert_eq!(
        output.pointer("/execution/modeChanged").and_then(Value::as_bool),
        Some(false),
        "{id} changed search mode and bypassed lexical scoring",
    );
}

fn validate_candidate_spec(spec: &CandidateSpec) {
    assert_eq!(spec.schema_version, 1);
    assert!(!spec.corpus_version.trim().is_empty());
    assert!(!spec.extensions.is_empty());
    assert!(!spec.queries.is_empty());
    let mut ids = HashSet::new();
    for query in &spec.queries {
        assert!(ids.insert(query.id.as_str()), "duplicate query id: {}", query.id);
        assert!(!query.query_class.trim().is_empty(), "{} has an empty query class", query.id);
        assert!(!query.intent.trim().is_empty(), "{} has an empty intent", query.id);
        validate_query_request(&query.id, &query.request);
    }
}

fn validate_spec(spec: &RelevanceSpec, corpus_root: &Path) {
    assert_eq!(spec.schema_version, 1);
    assert!(!spec.queries.is_empty());
    assert!(!spec.corpus_version.trim().is_empty());
    assert!(!spec.extensions.is_empty());
    let mut ids = HashSet::new();
    let mut class_counts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut global_negative_paths = HashSet::new();
    for negative in &spec.global_negatives {
        validate_relative_path(&negative.path, "global negative");
        assert!(!negative.reason.trim().is_empty(),
            "global negative {} has an empty reason", negative.path);
        assert!(global_negative_paths.insert(negative.path.as_str()),
            "duplicate global negative: {}", negative.path);
        assert!(corpus_root.join(&negative.path).is_file(),
            "global negative references missing {}", negative.path);
    }

    for query in &spec.queries {
        assert!(ids.insert(query.id.as_str()), "duplicate query id: {}", query.id);
        assert!(!query.intent.trim().is_empty(), "{} has an empty intent", query.id);
        *class_counts.entry(query.query_class.as_str()).or_default() += 1;
        validate_query_request(&query.id, &query.request);

        let mut judged_paths = HashSet::new();
        let mut negative_paths = HashSet::new();
        for negative in &query.negatives {
            validate_relative_path(&negative.path, &query.id);
            assert!(!negative.reason.trim().is_empty(),
                "{} has an empty negative reason", query.id);
            assert!(negative_paths.insert(negative.path.as_str()),
                "{} repeats negative {}", query.id, negative.path);
            assert!(!global_negative_paths.contains(negative.path.as_str()),
                "{} repeats global negative {}", query.id, negative.path);
            assert!(corpus_root.join(&negative.path).is_file(),
                "{} negative references missing {}", query.id, negative.path);
        }
        let mut has_primary = false;
        for judgment in &query.judgments {
            assert!((1..=3).contains(&judgment.grade),
                "{} has invalid grade {}", query.id, judgment.grade);
            assert!(!judgment.reason.trim().is_empty(), "{} has an empty reason", query.id);
            assert!(judged_paths.insert(judgment.path.as_str()),
                "{} repeats judgment {}", query.id, judgment.path);
            assert!(!negative_paths.contains(judgment.path.as_str()),
                "{} both judges and rejects {}", query.id, judgment.path);
            validate_relative_path(&judgment.path, &query.id);
            assert!(!global_negative_paths.contains(judgment.path.as_str()),
                "{} judges globally negative {}", query.id, judgment.path);
            assert!(corpus_root.join(&judgment.path).is_file(),
                "{} references missing {}", query.id, judgment.path);
            has_primary |= judgment.grade == 3;
        }
        assert!(has_primary, "{} has no grade-3 primary result", query.id);
    }

    assert!(!class_counts.is_empty());
}

fn validate_checked_fixture(spec: &RelevanceSpec) {
    validate_spec(spec, &fixture_root().join("corpus"));
    assert_eq!(spec.queries.len(), EXPECTED_QUERY_COUNT);
    let mut class_counts: BTreeMap<&str, usize> = BTreeMap::new();
    for query in &spec.queries {
        *class_counts.entry(query.query_class.as_str()).or_default() += 1;
    }
    assert_eq!(class_counts.len(), EXPECTED_CLASS_COUNT);
    for (query_class, count) in class_counts {
        assert_eq!(count, EXPECTED_QUERIES_PER_CLASS,
            "class {query_class} has {count} queries");
    }
}

fn build_context(
    extensions: &[String],
    corpus_root: &Path,
) -> (HandlerContext, PathBuf, tempfile::TempDir) {
    let corpus_root = crate::canonicalize_test_root(corpus_root);
    let extensions = extensions.join(",");
    let content_index = crate::build_content_index(&crate::ContentIndexArgs {
        dir: corpus_root.to_string_lossy().to_string(),
        ext: extensions.clone(),
        threads: 1,
        ..Default::default()
    }).expect("relevance corpus should index");
    let index_temp = tempfile::tempdir().expect("relevance index tempdir should be creatable");
    let index_root = crate::canonicalize_test_root(index_temp.path());
    let context = HandlerContextBuilder::new()
        .with_content_index(content_index)
        .with_server_dir(corpus_root.to_string_lossy().to_string())
        .with_server_ext(extensions)
        .with_index_base(index_root.join("runtime-index"))
        .with_max_response_bytes(1024 * 1024)
        .build();
    (context, corpus_root, index_temp)
}

fn relative_result_path(corpus_root: &Path, path: &str) -> String {
    let root = crate::clean_path(&corpus_root.to_string_lossy());
    let path = crate::clean_path(path);
    path.strip_prefix(&format!("{root}/"))
        .unwrap_or_else(|| panic!(
            "result path is outside the relevance corpus root: path={path}, root={root}"
        ))
        .to_string()
}

fn dcg_at(grades: &[u8], cutoff: usize) -> f64 {
    grades
        .iter()
        .take(cutoff)
        .enumerate()
        .map(|(index, &grade)| {
            let gain = (2_u32.pow(u32::from(grade)) - 1) as f64;
            gain / ((index + 2) as f64).log2()
        })
        .sum()
}

fn ndcg_at(retrieved_grades: &[u8], judged_grades: &[u8], cutoff: usize) -> f64 {
    let actual = dcg_at(retrieved_grades, cutoff);
    let mut ideal = judged_grades.to_vec();
    ideal.sort_unstable_by_key(|&grade| Reverse(grade));
    let ideal = dcg_at(&ideal, cutoff);
    if ideal == 0.0 { 0.0 } else { actual / ideal }
}

fn reciprocal_rank_at(grades: &[u8], cutoff: usize, minimum_grade: u8) -> f64 {
    grades
        .iter()
        .take(cutoff)
        .position(|&grade| grade >= minimum_grade)
        .map_or(0.0, |index| 1.0 / (index + 1) as f64)
}

fn recall_at(retrieved_paths: &[String], judgments: &[Judgment], cutoff: usize) -> f64 {
    if judgments.is_empty() {
        return 0.0;
    }
    let retrieved: HashSet<&str> = retrieved_paths
        .iter()
        .take(cutoff)
        .map(String::as_str)
        .collect();
    let found = judgments.iter()
        .filter(|judgment| retrieved.contains(judgment.path.as_str()))
        .count();
    found as f64 / judgments.len() as f64
}

fn round_metric(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}

fn summarize(queries: &[QueryQuality]) -> MetricSet {
    let query_count = queries.len();
    let divisor = query_count.max(1) as f64;
    MetricSet {
        query_count,
        ndcg_at_10: round_metric(queries.iter().map(|query| query.ndcg_at_10).sum::<f64>() / divisor),
        mrr_at_10: round_metric(queries.iter().map(|query| query.mrr_at_10).sum::<f64>() / divisor),
        recall_at_50: round_metric(queries.iter().map(|query| query.recall_at_50).sum::<f64>() / divisor),
        success_at_1: round_metric(queries.iter().filter(|query| query.success_at_1).count() as f64 / divisor),
        explicit_negative_hits_at_10: queries.iter()
            .map(|query| query.explicit_negative_hits_at_10)
            .sum(),
    }
}

fn aggregate_baseline(quality: &QualityReport) -> AggregateBaseline {
    AggregateBaseline {
        schema_version: quality.schema_version,
        corpus_version: quality.corpus_version.clone(),
        model: quality.model.clone(),
        metrics: quality.metrics.clone(),
        scored_metrics: quality.scored_metrics.clone(),
        per_class: quality.per_class.clone(),
        query_digest: quality.query_digest.clone(),
        corpus_digest: quality.corpus_digest.clone(),
    }
}

fn assert_metric_set_close(actual: &MetricSet, expected: &MetricSet, context: &str) {
    assert_eq!(actual.query_count, expected.query_count, "{context} query count");
    assert_eq!(actual.explicit_negative_hits_at_10, expected.explicit_negative_hits_at_10,
        "{context} explicit negative hits at 10");
    for (name, actual_value, expected_value) in [
        ("ndcgAt10", actual.ndcg_at_10, expected.ndcg_at_10),
        ("mrrAt10", actual.mrr_at_10, expected.mrr_at_10),
        ("recallAt50", actual.recall_at_50, expected.recall_at_50),
        ("successAt1", actual.success_at_1, expected.success_at_1),
    ] {
        assert!((actual_value - expected_value).abs() <= 0.000_001,
            "{context} {name}: expected {expected_value}, got {actual_value}");
    }
}

fn percentile(values: &[u128], quantile: f64) -> u128 {
    if values.is_empty() {
        return 0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let index = ((sorted.len() - 1) as f64 * quantile).round() as usize;
    sorted[index]
}

fn success_at_1(grades: &[u8]) -> bool {
    grades.first() == Some(&3)
}

fn validate_complete_ranked_response(output: &Value) -> Result<(), String> {
    // Checked queries are unscoped; this also protects private scoped manifests.
    if output.get("coverageWarning").is_some() {
        return Err(format!("incomplete index coverage: {output}"));
    }
    let returned_file_count = output.get("files")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let total_file_count = output.pointer("/summary/totalFiles")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    let auto_balance_dropped = output.pointer("/summary/autoBalance/droppedFiles")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    let expected_file_count = total_file_count
        .saturating_sub(auto_balance_dropped)
        .min(RECALL_CUTOFF);
    if returned_file_count != expected_file_count {
        return Err(format!(
            "expected {expected_file_count} ranked files, got {returned_file_count}: {output}"
        ));
    }
    Ok(())
}


fn evaluate_query(
    context: &HandlerContext,
    corpus_root: &Path,
    query: &RelevanceQuery,
    global_negative_paths: &HashSet<&str>,
    scoring_policy: RelevanceScoringPolicy,
) -> (QueryQuality, u128) {
    let scoring_model = scoring_policy.model_for_request(&query.request);
    let mut request = query.request.clone();
    let request_object = request.as_object_mut().expect("validated request object");
    request_object.insert("maxResults".to_string(), Value::from(RECALL_CUTOFF));
    request_object.insert("showLines".to_string(), Value::Bool(false));

    let started = Instant::now();
    let result = with_test_lexical_scoring_model(scoring_model, || {
        dispatch_tool(context, "xray_grep", &request)
    });
    let elapsed_micros = started.elapsed().as_micros();
    assert!(!result.is_error, "{} failed: {}", query.id, result.content[0].text);
    let output: Value = serde_json::from_str(&result.content[0].text)
        .unwrap_or_else(|error| panic!("{} returned invalid JSON: {error}", query.id));
    validate_model_sensitive_response(&query.id, scoring_policy, &output);
    let search_mode = output.pointer("/summary/searchMode")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{} response has no searchMode: {output}", query.id))
        .to_string();
    validate_complete_ranked_response(&output)
        .unwrap_or_else(|error| panic!("{} response is incomplete: {error}", query.id));
    let retrieved_paths: Vec<String> = output.get("files")
        .and_then(Value::as_array)
        .map(|files| {
            files.iter()
                .filter_map(|file| file.get("path").and_then(Value::as_str))
                .map(|path| relative_result_path(corpus_root, path))
                .collect()
        })
        .unwrap_or_default();

    let grades_by_path: HashMap<&str, u8> = query.judgments.iter()
        .map(|judgment| (judgment.path.as_str(), judgment.grade))
        .collect();
    let query_negative_paths: HashSet<&str> = query.negatives.iter()
        .map(|negative| negative.path.as_str())
        .collect();
    for path in retrieved_paths.iter().take(QUALITY_CUTOFF) {
        assert!(grades_by_path.contains_key(path.as_str())
            || query_negative_paths.contains(path.as_str())
            || global_negative_paths.contains(path.as_str()),
            "{} has unlabelled top-{} result: {}",
            query.id, QUALITY_CUTOFF, path);
    }
    let explicit_negative_hits_at_10 = retrieved_paths.iter()
        .take(QUALITY_CUTOFF)
        .filter(|path| query_negative_paths.contains(path.as_str())
            || global_negative_paths.contains(path.as_str()))
        .count();
    let retrieved_grades: Vec<u8> = retrieved_paths.iter()
        .map(|path| grades_by_path.get(path.as_str()).copied().unwrap_or(0))
        .collect();
    let judged_grades: Vec<u8> = query.judgments.iter()
        .map(|judgment| judgment.grade)
        .collect();
    let retrieved_set: HashSet<&str> = retrieved_paths.iter().map(String::as_str).collect();
    let mut missing_judgments: Vec<String> = query.judgments.iter()
        .filter(|judgment| !retrieved_set.contains(judgment.path.as_str()))
        .map(|judgment| judgment.path.clone())
        .collect();
    missing_judgments.sort();

    let quality = QueryQuality {
        id: query.id.clone(),
        query_class: query.query_class.clone(),
        search_mode,
        ndcg_at_10: round_metric(ndcg_at(&retrieved_grades, &judged_grades, QUALITY_CUTOFF)),
        mrr_at_10: round_metric(reciprocal_rank_at(&retrieved_grades, QUALITY_CUTOFF, USEFUL_GRADE)),
        recall_at_50: round_metric(recall_at(&retrieved_paths, &query.judgments, RECALL_CUTOFF)),
        success_at_1: success_at_1(&retrieved_grades),
        explicit_negative_hits_at_10,
        top_paths: retrieved_paths.into_iter().take(QUALITY_CUTOFF).collect(),
        missing_judgments,
    };
    (quality, elapsed_micros)
}

fn collect_candidates(
    spec: &CandidateSpec,
    corpus_root: &Path,
    model: &str,
    scoring_policy: impl Into<RelevanceScoringPolicy>,
) -> CandidateReport {
    let scoring_policy = scoring_policy.into();
    validate_candidate_spec(spec);
    let (context, corpus_root, _index_temp) = build_context(&spec.extensions, corpus_root);
    let digest = corpus_digest(&corpus_root, &spec.extensions);
    collect_candidates_with_context(
        spec,
        &context,
        &corpus_root,
        model,
        scoring_policy,
        &digest,
    )
}


fn collect_candidates_with_context(
    spec: &CandidateSpec,
    context: &HandlerContext,
    corpus_root: &Path,
    model: &str,
    scoring_policy: RelevanceScoringPolicy,
    corpus_digest: &str,
) -> CandidateReport {
    assert!(!model.trim().is_empty(), "candidate model label cannot be empty");
    validate_model_sensitive_queries(
        [scoring_policy],
        spec.queries.iter().map(|query| (query.id.as_str(), &query.request)),
    );
    let mut query_reports = Vec::with_capacity(spec.queries.len());
    for query in &spec.queries {
        let scoring_model = scoring_policy.model_for_request(&query.request);
        let mut request = query.request.clone();
        let request_object = request.as_object_mut().expect("validated request object");
        request_object.insert("maxResults".to_string(), Value::from(RECALL_CUTOFF));
        request_object.insert("showLines".to_string(), Value::Bool(false));
        let result = with_test_lexical_scoring_model(scoring_model, || {
            dispatch_tool(context, "xray_grep", &request)
        });
        assert!(!result.is_error, "{} failed: {}", query.id, result.content[0].text);
        let output: Value = serde_json::from_str(&result.content[0].text)
            .unwrap_or_else(|error| panic!("{} returned invalid JSON: {error}", query.id));
        validate_model_sensitive_response(&query.id, scoring_policy, &output);
        validate_complete_ranked_response(&output)
            .unwrap_or_else(|error| panic!("{} response is incomplete: {error}", query.id));
        let search_mode = output.pointer("/summary/searchMode")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("{} response has no searchMode: {output}", query.id))
            .to_string();
        let mut candidates: Vec<String> = output.get("files")
            .and_then(Value::as_array)
            .map(|files| {
                files.iter()
                    .filter_map(|file| file.get("path").and_then(Value::as_str))
                    .map(|path| relative_result_path(corpus_root, path))
                    .collect()
            })
            .unwrap_or_default();
        let candidate_count = candidates.len();
        candidates.sort();
        candidates.dedup();
        assert_eq!(candidates.len(), candidate_count,
            "{} returned duplicate candidate paths", query.id);
        query_reports.push(QueryCandidates {
            id: query.id.clone(),
            query_class: query.query_class.clone(),
            intent: query.intent.clone(),
            request: query.request.clone(),
            search_mode,
            candidates,
        });
    }
    let candidate_bytes = serde_json::to_vec(&query_reports)
        .expect("candidate digest projection should serialize");
    CandidateReport {
        schema_version: CANDIDATE_REPORT_SCHEMA_VERSION,
        corpus_version: spec.corpus_version.clone(),
        model: model.to_string(),
        candidate_digest: format!("{:016x}", code_xray::stable_hash(&[&candidate_bytes])),
        corpus_digest: corpus_digest.to_string(),
        queries: query_reports,
    }
}

fn parse_relevance_scorer(value: &str) -> Result<RelevanceScorerConfig, String> {
    match value {
        "tfidf-file-stem-v1" => Ok(RelevanceScorerConfig {
            scoring_policy: RelevanceScoringPolicy::Fixed(
                LexicalScoringModel::TfIdfFileStem,
            ),
            default_label: "tfidf-file-stem-v1".to_string(),
        }),
        "smoothed-tfidf-file-stem-v2" => Ok(RelevanceScorerConfig {
            scoring_policy: RelevanceScoringPolicy::Fixed(
                LexicalScoringModel::SmoothedTfIdf,
            ),
            default_label: "smoothed-tfidf-file-stem-v2".to_string(),
        }),
        HYBRID_RELEVANCE_MODEL => Ok(RelevanceScorerConfig {
            scoring_policy: RelevanceScoringPolicy::HybridBm25OrTfidfAnd,
            default_label: HYBRID_RELEVANCE_MODEL.to_string(),
        }),
        _ => {
            let parameters = value.strip_prefix("bm25:")
                .ok_or_else(|| format!("unsupported relevance scorer: {value}"))?;
            let mut k1 = None;
            let mut b = None;
            for parameter in parameters.split(',') {
                let (name, raw_value) = parameter.split_once('=')
                    .ok_or_else(|| format!("invalid BM25 parameter: {parameter}"))?;
                let parsed = raw_value.parse::<f64>()
                    .map_err(|_| format!("invalid BM25 value: {raw_value}"))?;
                match name {
                    "k1" if k1.replace(parsed).is_none() => {},
                    "b" if b.replace(parsed).is_none() => {},
                    "k1" | "b" => return Err(format!("duplicate BM25 parameter: {name}")),
                    _ => return Err(format!("unknown BM25 parameter: {name}")),
                }
            }
            let k1 = k1.ok_or_else(|| "BM25 requires k1".to_string())?;
            let b = b.ok_or_else(|| "BM25 requires b".to_string())?;
            if !k1.is_finite() || k1 <= 0.0 {
                return Err("BM25 k1 must be finite and greater than zero".to_string());
            }
            if !b.is_finite() || !(0.0..=1.0).contains(&b) {
                return Err("BM25 b must be finite and between zero and one".to_string());
            }
            Ok(RelevanceScorerConfig {
                scoring_policy: RelevanceScoringPolicy::Fixed(
                    LexicalScoringModel::Bm25 { k1, b },
                ),
                default_label: format!("bm25-file-stem-v2-k1-{k1}-b-{b}"),
            })
        }
    }
}

fn parse_relevance_scorer_matrix(value: &str) -> Result<Vec<RelevanceScorerConfig>, String> {
    let mut labels = HashSet::new();
    let mut scorers = Vec::new();
    for raw_scorer in value.split(';').map(str::trim).filter(|value| !value.is_empty()) {
        let scorer = parse_relevance_scorer(raw_scorer)?;
        if !labels.insert(scorer.default_label.clone()) {
            return Err(format!("duplicate relevance scorer: {}", scorer.default_label));
        }
        scorers.push(scorer);
    }
    if scorers.is_empty() {
        return Err("XRAY_RELEVANCE_SCORERS must contain at least one scorer".to_string());
    }
    Ok(scorers)
}


fn relevance_scorer_from_env_value(
    value: Result<String, std::env::VarError>,
) -> RelevanceScorerConfig {
    match value {
        Ok(value) => parse_relevance_scorer(&value)
            .unwrap_or_else(|error| panic!("invalid XRAY_RELEVANCE_SCORER: {error}")),
        Err(std::env::VarError::NotPresent) => {
            parse_relevance_scorer(PRODUCTION_RELEVANCE_MODEL).unwrap()
        }
        Err(std::env::VarError::NotUnicode(_)) => {
            panic!("XRAY_RELEVANCE_SCORER must be valid Unicode")
        }
    }
}

fn relevance_scorer_from_env() -> RelevanceScorerConfig {
    relevance_scorer_from_env_value(std::env::var("XRAY_RELEVANCE_SCORER"))
}


fn relevance_model_from_env_value(
    value: Result<String, std::env::VarError>,
    scorer: &RelevanceScorerConfig,
) -> String {
    match value {
        Ok(model) => {
            assert!(
                scorer.scoring_policy.is_production() || model == scorer.default_label,
                "XRAY_RELEVANCE_MODEL cannot override an experimental scorer label",
            );
            model
        }
        Err(std::env::VarError::NotPresent) => scorer.default_label.clone(),
        Err(std::env::VarError::NotUnicode(_)) => {
            panic!("XRAY_RELEVANCE_MODEL must be valid Unicode")
        }
    }
}

fn relevance_model_from_env(scorer: &RelevanceScorerConfig) -> String {
    relevance_model_from_env_value(std::env::var("XRAY_RELEVANCE_MODEL"), scorer)
}

fn run_evaluation(
    spec: &RelevanceSpec,
    corpus_root: &Path,
    warm_up: bool,
    model: &str,
    scoring_policy: impl Into<RelevanceScoringPolicy>,
) -> OfflineReport {
    let scoring_policy = scoring_policy.into();
    validate_spec(spec, corpus_root);
    let (context, corpus_root, _index_temp) = build_context(&spec.extensions, corpus_root);
    let digest = corpus_digest(&corpus_root, &spec.extensions);
    run_evaluation_with_context(
        spec,
        &context,
        &corpus_root,
        warm_up,
        model,
        scoring_policy,
        &digest,
    )
}

fn run_evaluation_with_context(
    spec: &RelevanceSpec,
    context: &HandlerContext,
    corpus_root: &Path,
    warm_up: bool,
    model: &str,
    scoring_policy: RelevanceScoringPolicy,
    corpus_digest: &str,
) -> OfflineReport {
    assert!(!model.trim().is_empty(), "relevance model label cannot be empty");
    validate_model_sensitive_queries(
        [scoring_policy],
        spec.queries.iter().map(|query| (query.id.as_str(), &query.request)),
    );
    let global_negative_paths: HashSet<&str> = spec.global_negatives.iter()
        .map(|negative| negative.path.as_str())
        .collect();
    if warm_up {
        for query in &spec.queries {
            let _ = evaluate_query(
                context,
                corpus_root,
                query,
                &global_negative_paths,
                scoring_policy,
            );
        }
    }

    let mut query_reports = Vec::with_capacity(spec.queries.len());
    let mut query_latencies = Vec::with_capacity(spec.queries.len());
    for query in &spec.queries {
        let (quality, elapsed_micros) =
            evaluate_query(
                context,
                corpus_root,
                query,
                &global_negative_paths,
                scoring_policy,
            );
        query_reports.push(quality);
        query_latencies.push(QueryLatency {
            id: query.id.clone(),
            query_class: query.query_class.clone(),
            micros: elapsed_micros,
        });
    }
    let latencies: Vec<u128> = query_latencies.iter().map(|sample| sample.micros).collect();

    let mut grouped: BTreeMap<String, Vec<QueryQuality>> = BTreeMap::new();
    for query in &query_reports {
        grouped.entry(query.query_class.clone()).or_default().push(query.clone());
    }
    let per_class = grouped.into_iter()
        .map(|(query_class, queries)| (query_class, summarize(&queries)))
        .collect();
    let scored_query_reports: Vec<QueryQuality> = query_reports.iter()
        .filter(|report| uses_tfidf_search_mode(&report.search_mode))
        .cloned()
        .collect();
    let scored_metrics = summarize(&scored_query_reports);
    let corpus_digest = corpus_digest.to_string();
    let digest_projection: Vec<Value> = spec.queries.iter().zip(&query_reports)
        .map(|(query, result)| {
            serde_json::json!({
                "id": query.id,
                "queryClass": query.query_class,
                "intent": query.intent,
                "searchMode": result.search_mode,
                "request": query.request,
                "judgments": query.judgments,
                "negatives": query.negatives,
                "successAt1": result.success_at_1,
                "topPaths": result.top_paths,
                "missingJudgments": result.missing_judgments,
            })
        })
        .collect();
    let digest_input = serde_json::json!({
        "globalNegatives": spec.global_negatives,
        "queries": digest_projection,
    });
    let query_bytes = serde_json::to_vec(&digest_input)
        .expect("query digest projection should serialize");
    let query_digest = format!("{:016x}", code_xray::stable_hash(&[&query_bytes]));

    let quality = QualityReport {
        schema_version: REPORT_SCHEMA_VERSION,
        corpus_version: spec.corpus_version.clone(),
        model: model.to_string(),
        metrics: summarize(&query_reports),
        scored_metrics,
        per_class,
        query_digest,
        corpus_digest,
        queries: query_reports,
    };
    query_latencies.sort_by_key(|sample| Reverse(sample.micros));
    query_latencies.truncate(5);

    let latency = LatencySummary {
        samples: latencies.len(),
        p50_micros: percentile(&latencies, 0.50),
        p95_micros: percentile(&latencies, 0.95),
        max_micros: latencies.iter().copied().max().unwrap_or(0),
        slowest_queries: query_latencies,
    };
    OfflineReport { quality, latency }
}

#[test]
fn relevance_response_completeness_guard_handles_caps() {
    let balanced = serde_json::json!({
        "files": [{}, {}],
        "summary": {
            "totalFiles": 5,
            "autoBalance": { "droppedFiles": 3 }
        }
    });
    assert!(validate_complete_ranked_response(&balanced).is_ok());

    let capped = serde_json::json!({
        "files": (0..RECALL_CUTOFF).map(|_| serde_json::json!({})).collect::<Vec<_>>(),
        "summary": { "totalFiles": 75 }
    });
    assert!(validate_complete_ranked_response(&capped).is_ok());

    let omitted = serde_json::json!({
        "files": [{}],
        "summary": { "totalFiles": 2 }
    });
    assert!(validate_complete_ranked_response(&omitted).is_err());

    let incomplete_coverage = serde_json::json!({
        "files": [],
        "summary": { "totalFiles": 0 },
        "coverageWarning": { "reason": "extension_not_indexed" }
    });
    assert!(validate_complete_ranked_response(&incomplete_coverage).is_err());
}


#[test]
fn relevance_metrics_match_hand_calculated_examples() {
    let grades = [3, 0, 2, 1];
    assert!((dcg_at(&grades, 4) - 8.930_676_558).abs() < 1e-9);
    assert!((ndcg_at(&grades, &grades, 4) - 0.950_801_334).abs() < 1e-9);
    assert!(ndcg_at(&[3], &[3, 2], 4) < 1.0);
    assert_eq!(reciprocal_rank_at(&grades, 10, 2), 1.0);
    assert_eq!(reciprocal_rank_at(&[0, 0, 2], 2, 2), 0.0);

    let judgments = vec![
        Judgment { path: "a".to_string(), grade: 3, reason: "primary".to_string() },
        Judgment { path: "b".to_string(), grade: 2, reason: "useful".to_string() },
        Judgment { path: "c".to_string(), grade: 1, reason: "context".to_string() },
    ];
    assert!((recall_at(&["a".to_string(), "c".to_string()], &judgments, 10) - 2.0 / 3.0).abs() < 1e-9);
    assert_eq!(round_metric(1.0 / 3.0), 0.333_333);
    assert_eq!(percentile(&[10, 20, 30, 40, 50], 0.50), 30);
    assert_eq!(percentile(&[10, 20, 30, 40, 50], 0.95), 50);
    assert_eq!(percentile(&[], 0.95), 0);
    assert!(success_at_1(&[3, 0]));
    assert!(!success_at_1(&[2, 3]));
    assert!(!success_at_1(&[]));
}

#[test]
fn relevance_corpus_digest_changes_with_content_and_extensions() {
    let temp = tempfile::tempdir().unwrap();
    let root = crate::canonicalize_test_root(temp.path());
    fs::write(root.join("sample.rs"), "first\n").unwrap();
    let extensions = vec!["rs".to_string()];
    let first = corpus_digest(&root, &extensions);
    fs::write(root.join("sample.rs"), "second\n").unwrap();
    let second = corpus_digest(&root, &extensions);
    let third = corpus_digest(&root, &["rs".to_string(), "md".to_string()]);
    assert_ne!(first, second);
    assert_ne!(second, third);
}

#[test]
fn relevance_negative_policies_accept_explicit_zero_grade_results() {
    let temp = tempfile::tempdir().unwrap();
    let root = crate::canonicalize_test_root(temp.path());
    fs::write(root.join("a_main.rs"), "needle\n").unwrap();
    fs::write(root.join("c_global_noise.rs"), "needle\n").unwrap();
    let query_negative_paths = [
        "b_query_noise.rs",
        "d_query_noise.rs",
        "e_query_noise.rs",
        "f_query_noise.rs",
        "g_query_noise.rs",
        "h_query_noise.rs",
        "i_query_noise.rs",
        "j_query_noise.rs",
        "k_query_noise.rs",
    ];
    for path in query_negative_paths {
        fs::write(root.join(path), "needle\n").unwrap();
    }

    let spec = RelevanceSpec {
        schema_version: 1,
        corpus_version: "negative-policy-test".to_string(),
        extensions: vec!["rs".to_string()],
        global_negatives: vec![ExplicitNegative {
            path: "c_global_noise.rs".to_string(),
            reason: "global noise".to_string(),
        }],
        queries: vec![RelevanceQuery {
            id: "negative-policy".to_string(),
            query_class: "test".to_string(),
            intent: "find the primary needle artifact".to_string(),
            request: serde_json::json!({"terms": ["needle"], "substring": false}),
            judgments: vec![Judgment {
                path: "a_main.rs".to_string(),
                grade: 3,
                reason: "primary".to_string(),
            }],
            negatives: query_negative_paths.into_iter().map(|path| ExplicitNegative {
                path: path.to_string(),
                reason: "query-specific noise".to_string(),
            }).collect(),
        }],
    };

    let report = run_evaluation(
        &spec,
        &root,
        false,
        PRODUCTION_RELEVANCE_MODEL,
        LexicalScoringModel::TfIdfFileStem,
    ).quality;
    assert_eq!(report.metrics.query_count, 1);
    // Equal scores make path order the deterministic primary-result oracle.
    assert_eq!(report.metrics.success_at_1, 1.0);
    assert_eq!(report.metrics.explicit_negative_hits_at_10, 9);
    assert_eq!(report.queries[0].explicit_negative_hits_at_10, 9);
    assert_eq!(report.queries[0].top_paths.len(), QUALITY_CUTOFF);
    assert_eq!(report.queries[0].top_paths.last().unwrap(), "j_query_noise.rs");
}


#[test]
fn relevance_report_output_path_rejects_tracked_workspace_paths() {
    let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    assert!(validate_report_output_path(
        &repository_root.join("target/relevance/report.json")
    ).is_ok());
    assert!(validate_report_output_path(
        &repository_root.join("target/../CHANGELOG.md")
    ).is_err());
    assert!(validate_report_output_path(
        &repository_root.join("benches/fixtures/relevance/baseline-tfidf.json")
    ).is_err());
    assert!(validate_report_output_path(
        &repository_root.parent().unwrap().join("private-relevance/report.json")
    ).is_ok());

    #[cfg(windows)]
    {
        let lowercase_repo = PathBuf::from(repository_root.to_string_lossy().to_lowercase());
        assert!(validate_report_output_path(
            &lowercase_repo.join("benches/fixtures/relevance/baseline-tfidf.json")
        ).is_err());
    }

    #[cfg(unix)]
    {
        let target_root = repository_root.join("target");
        let temp = tempfile::Builder::new()
            .prefix("relevance-path-guard-")
            .tempdir_in(&target_root)
            .unwrap();
        let link = temp.path().join("outside-target");
        std::os::unix::fs::symlink(repository_root.join("benches"), &link).unwrap();
        let error = validate_report_output_path(&link.join("report.json")).unwrap_err();
        assert!(error.contains("physical"), "{error}");
    }
}


#[test]
fn relevance_manifest_is_valid_and_balanced() {
    validate_checked_fixture(&load_spec());
}

#[test]
fn current_tfidf_matches_checked_relevance_baseline() {
    let spec = load_spec();
    let corpus_root = fixture_root().join("corpus");
    let report = run_evaluation(
        &spec,
        &corpus_root,
        false,
        PRODUCTION_RELEVANCE_MODEL,
        LexicalScoringModel::TfIdfFileStem,
    ).quality;
    assert_eq!(report.scored_metrics.query_count, EXPECTED_TFIDF_QUERY_COUNT);
    let warm_report = run_evaluation(
        &spec,
        &corpus_root,
        true,
        PRODUCTION_RELEVANCE_MODEL,
        LexicalScoringModel::TfIdfFileStem,
    ).quality;
    assert_eq!(
        aggregate_baseline(&report),
        aggregate_baseline(&warm_report),
        "cold and warm relevance quality must match"
    );
    let baseline = load_baseline();
    assert_eq!(report.schema_version, baseline.schema_version);
    assert_eq!(report.corpus_version, baseline.corpus_version);
    assert_eq!(report.model, baseline.model);
    assert!(report.queries.iter().all(|query| query.missing_judgments.is_empty()),
        "checked relevance judgments must all be lexically reachable");
    assert_metric_set_close(&report.metrics, &baseline.metrics, "overall");
    assert_metric_set_close(&report.scored_metrics, &baseline.scored_metrics, "scored queries");
    assert_eq!(report.per_class.keys().collect::<Vec<_>>(),
        baseline.per_class.keys().collect::<Vec<_>>());
    for (query_class, actual) in &report.per_class {
        assert_metric_set_close(actual, &baseline.per_class[query_class], query_class);
    }
    assert_eq!(
        report.corpus_digest,
        baseline.corpus_digest,
        "corpus digest changed; inspect corpus file names/bytes and indexed extensions"
    );
    assert_eq!(
        report.query_digest,
        baseline.query_digest,
        "query digest changed; inspect generated query metadata and ranking evidence"
    );
}

#[test]
fn relevance_candidate_report_is_rank_blind() {
    let temp = tempfile::tempdir().unwrap();
    let root = crate::canonicalize_test_root(temp.path());
    fs::write(root.join("a_noise.rs"), "needle filler filler filler\n").unwrap();
    fs::write(root.join("m_unrelated.rs"), "filler filler filler filler\n").unwrap();
    fs::write(root.join("z_primary.rs"), "needle needle needle filler\n").unwrap();
    let spec = CandidateSpec {
        schema_version: 1,
        corpus_version: "candidate-test".to_string(),
        extensions: vec!["rs".to_string()],
        queries: vec![CandidateQuery {
            id: "candidate-test".to_string(),
            query_class: "test".to_string(),
            intent: "find the needle implementation".to_string(),
            request: serde_json::json!({"terms": ["needle"], "substring": false}),
        }],
    };

    let (context, corpus_root, _index_temp) = build_context(&spec.extensions, &root);
    let result = dispatch_tool(&context, "xray_grep", &serde_json::json!({
        "terms": ["needle"],
        "substring": false,
        "maxResults": RECALL_CUTOFF,
        "showLines": false,
    }));
    let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
    let ranked_paths: Vec<String> = output["files"].as_array().unwrap().iter()
        .map(|file| relative_result_path(&corpus_root, file["path"].as_str().unwrap()))
        .collect();
    assert_eq!(ranked_paths, vec!["z_primary.rs", "a_noise.rs"]);

    let report = collect_candidates(
        &spec,
        &root,
        PRODUCTION_RELEVANCE_MODEL,
        LexicalScoringModel::TfIdfFileStem,
    );
    let repeated_report = collect_candidates(
        &spec,
        &root,
        "candidate-model-v2",
        LexicalScoringModel::TfIdfFileStem,
    );
    assert_eq!(report.schema_version, CANDIDATE_REPORT_SCHEMA_VERSION);
    assert_eq!(report.model, PRODUCTION_RELEVANCE_MODEL);
    assert_eq!(repeated_report.model, "candidate-model-v2");
    assert_eq!(report.candidate_digest, repeated_report.candidate_digest);
    assert_eq!(report.queries[0].candidates, vec!["a_noise.rs", "z_primary.rs"]);
    assert_eq!(report.queries[0].candidates, repeated_report.queries[0].candidates);
    let report_value = serde_json::to_value(&report).unwrap();
    let mut report_keys: Vec<&str> = report_value.as_object().unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    report_keys.sort_unstable();
    assert_eq!(report_keys, [
        "candidateDigest", "corpusDigest", "corpusVersion", "model", "queries",
        "schemaVersion",
    ]);
    let mut query_keys: Vec<&str> = report_value["queries"][0].as_object().unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    query_keys.sort_unstable();
    assert_eq!(query_keys, [
        "candidates", "id", "intent", "queryClass", "request", "searchMode",
    ]);

    fs::write(root.join("b_context.rs"), "needle filler filler\n").unwrap();
    let changed_report = collect_candidates(
        &spec,
        &root,
        "candidate-model-v3",
        LexicalScoringModel::TfIdfFileStem,
    );
    assert_ne!(report.candidate_digest, changed_report.candidate_digest);
    assert_eq!(changed_report.queries[0].candidates,
        vec!["a_noise.rs", "b_context.rs", "z_primary.rs"]);
}

#[test]
fn relevance_hybrid_policy_routes_only_validated_or_modes_to_bm25() {
    let policy = RelevanceScoringPolicy::HybridBm25OrTfidfAnd;
    let bm25 = LexicalScoringModel::Bm25 { k1: 2.0, b: 0.5 };
    assert_eq!(
        policy.model_for_request(&serde_json::json!({"terms": ["needle"]})),
        bm25,
    );
    assert_eq!(
        policy.model_for_request(&serde_json::json!({
            "terms": ["needle"],
            "substring": false,
        })),
        bm25,
    );
    for request in [
        serde_json::json!({"terms": ["needle", "marker"], "mode": "and"}),
        serde_json::json!({"terms": ["needle.*"], "regex": true}),
        serde_json::json!({"terms": ["needle marker"], "phrase": true}),
        serde_json::json!({"terms": ["needle.*"], "lineRegex": true}),
    ] {
        assert_eq!(
            policy.model_for_request(&request),
            LexicalScoringModel::TfIdfFileStem,
        );
    }
}

#[test]
fn relevance_scorer_override_changes_dispatch_ranking() {
    let temp = tempfile::tempdir().unwrap();
    let root = crate::canonicalize_test_root(temp.path());
    fs::write(root.join("a_short.rs"), "needle marker\n").unwrap();
    fs::write(root.join("m_unrelated.rs"), "filler\n").unwrap();
    fs::write(
        root.join("z_long.rs"),
        format!(
            "{}marker {}\n",
            "needle ".repeat(20),
            "filler ".repeat(80),
        ),
    ).unwrap();
    let extensions = vec!["rs".to_string()];
    let (context, corpus_root, _index_temp) = build_context(&extensions, &root);
    let request = serde_json::json!({
        "terms": ["needle"],
        "substring": false,
        "maxResults": 2,
        "showLines": false,
    });
    let ranked_paths = |policy: RelevanceScoringPolicy, request: &Value| {
        let model = policy.model_for_request(request);
        let result = with_test_lexical_scoring_model(model, || {
            dispatch_tool(&context, "xray_grep", request)
        });
        assert!(!result.is_error, "{}", result.content[0].text);
        let output: Value = serde_json::from_str(&result.content[0].text).unwrap();
        validate_model_sensitive_response("dispatch-ranking", policy, &output);
        output["files"].as_array().unwrap().iter()
            .map(|file| {
                relative_result_path(&corpus_root, file["path"].as_str().unwrap())
            })
            .collect::<Vec<_>>()
    };

    assert_eq!(
        ranked_paths(
            RelevanceScoringPolicy::Fixed(LexicalScoringModel::TfIdfFileStem),
            &request,
        ),
        vec!["a_short.rs", "z_long.rs"],
    );
    assert_eq!(
        ranked_paths(
            RelevanceScoringPolicy::Fixed(
                LexicalScoringModel::Bm25 { k1: 2.0, b: 0.5 },
            ),
            &request,
        ),
        vec!["z_long.rs", "a_short.rs"],
    );
    let hybrid = RelevanceScoringPolicy::HybridBm25OrTfidfAnd;
    assert_eq!(
        ranked_paths(hybrid, &request),
        vec!["z_long.rs", "a_short.rs"],
    );
    let and_request = serde_json::json!({
        "terms": ["needle", "marker"],
        "mode": "and",
        "substring": false,
    });
    assert_eq!(
        ranked_paths(hybrid, &and_request),
        vec!["a_short.rs", "z_long.rs"],
    );
}

#[test]
fn relevance_scorer_rejects_effective_mode_changes() {
    let temp = tempfile::tempdir().unwrap();
    let root = crate::canonicalize_test_root(temp.path());
    fs::write(root.join("primary.rs"), "needle value\n").unwrap();
    let spec = CandidateSpec {
        schema_version: 1,
        corpus_version: "effective-mode-test".to_string(),
        extensions: vec!["rs".to_string()],
        queries: vec![CandidateQuery {
            id: "effective-mode".to_string(),
            query_class: "test".to_string(),
            intent: "find the exact value".to_string(),
            request: serde_json::json!({"terms": ["needle value"]}),
        }],
    };

    let production = collect_candidates(
        &spec,
        &root,
        PRODUCTION_RELEVANCE_MODEL,
        LexicalScoringModel::TfIdfFileStem,
    );
    assert_eq!(production.queries[0].search_mode, "phrase");
    let panic = std::panic::catch_unwind(|| {
        collect_candidates(
            &spec,
            &root,
            "bm25-file-stem-v2-k1-2-b-0.5",
            LexicalScoringModel::Bm25 { k1: 2.0, b: 0.5 },
        );
    }).unwrap_err();
    assert!(panic_message(panic).contains("changed search mode"));
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else {
        "non-string panic".to_string()
    }
}

#[test]
fn relevance_model_env_routing_is_explicit() {
    let production = RelevanceScorerConfig {
        scoring_policy: RelevanceScoringPolicy::Fixed(
            LexicalScoringModel::TfIdfFileStem,
        ),
        default_label: "fallback-model".to_string(),
    };
    assert_eq!(
        relevance_model_from_env_value(Err(std::env::VarError::NotPresent), &production),
        "fallback-model",
    );
    assert_eq!(
        relevance_model_from_env_value(Ok("candidate-model-v2".to_string()), &production),
        "candidate-model-v2",
    );

    let experimental = parse_relevance_scorer("bm25:k1=1.2,b=0.75").unwrap();
    let panic = std::panic::catch_unwind(|| relevance_model_from_env_value(
        Ok("tfidf-file-stem-v1".to_string()),
        &experimental,
    )).unwrap_err();
    assert!(panic_message(panic).contains("cannot override"));

    let hybrid = parse_relevance_scorer(HYBRID_RELEVANCE_MODEL).unwrap();
    let panic = std::panic::catch_unwind(|| relevance_model_from_env_value(
        Ok("tfidf-file-stem-v1".to_string()),
        &hybrid,
    )).unwrap_err();
    assert!(panic_message(panic).contains("cannot override"));

    let panic = std::panic::catch_unwind(|| relevance_model_from_env_value(
        Err(std::env::VarError::NotUnicode(std::ffi::OsString::from("invalid"))),
        &production,
    )).unwrap_err();
    assert!(panic_message(panic).contains("must be valid Unicode"));
}

#[test]
fn relevance_scorer_validation_rejects_model_insensitive_modes() {
    for field in ["phrase", "lineRegex"] {
        let mut request = serde_json::json!({"terms": ["needle"]});
        request.as_object_mut().unwrap().insert(field.to_string(), Value::Bool(true));
        validate_query_request("query", &request);
        validate_model_sensitive_queries(
            [LexicalScoringModel::TfIdfFileStem],
            std::iter::once(("query", &request)),
        );
        for policy in [
            RelevanceScoringPolicy::Fixed(LexicalScoringModel::SmoothedTfIdf),
            RelevanceScoringPolicy::HybridBm25OrTfidfAnd,
        ] {
            let panic = std::panic::catch_unwind(|| {
                validate_model_sensitive_queries(
                    [policy],
                    std::iter::once(("query", &request)),
                );
            }).unwrap_err();
            assert!(panic_message(panic).contains(field));
        }
    }

    let changed_output = serde_json::json!({"execution": {"modeChanged": true}});
    validate_model_sensitive_response(
        "query",
        LexicalScoringModel::TfIdfFileStem,
        &changed_output,
    );
    for policy in [
        RelevanceScoringPolicy::Fixed(
            LexicalScoringModel::Bm25 { k1: 2.0, b: 0.5 },
        ),
        RelevanceScoringPolicy::HybridBm25OrTfidfAnd,
    ] {
        let panic = std::panic::catch_unwind(|| {
            validate_model_sensitive_response("query", policy, &changed_output);
        }).unwrap_err();
        assert!(panic_message(panic).contains("changed search mode"));
    }
}

#[test]
fn relevance_scorer_parsing_is_explicit() {
    assert_eq!(
        relevance_scorer_from_env_value(Err(std::env::VarError::NotPresent)),
        RelevanceScorerConfig {
            scoring_policy: RelevanceScoringPolicy::Fixed(
                LexicalScoringModel::TfIdfFileStem,
            ),
            default_label: PRODUCTION_RELEVANCE_MODEL.to_string(),
        },
    );
    assert_eq!(
        parse_relevance_scorer("smoothed-tfidf-file-stem-v2").unwrap(),
        RelevanceScorerConfig {
            scoring_policy: RelevanceScoringPolicy::Fixed(
                LexicalScoringModel::SmoothedTfIdf,
            ),
            default_label: "smoothed-tfidf-file-stem-v2".to_string(),
        },
    );
    assert_eq!(
        parse_relevance_scorer("bm25:k1=1.2,b=0.75").unwrap(),
        RelevanceScorerConfig {
            scoring_policy: RelevanceScoringPolicy::Fixed(
                LexicalScoringModel::Bm25 { k1: 1.2, b: 0.75 },
            ),
            default_label: "bm25-file-stem-v2-k1-1.2-b-0.75".to_string(),
        },
    );
    assert_eq!(
        parse_relevance_scorer(HYBRID_RELEVANCE_MODEL).unwrap(),
        RelevanceScorerConfig {
            scoring_policy: RelevanceScoringPolicy::HybridBm25OrTfidfAnd,
            default_label: HYBRID_RELEVANCE_MODEL.to_string(),
        },
    );

    assert!(parse_relevance_scorer("bm25:k1=1.2,b=1.1").is_err());
    assert!(parse_relevance_scorer("bm25:k1=0,b=0.75").is_err());
    assert!(parse_relevance_scorer("bm25:k1=1.2").is_err());
    assert!(parse_relevance_scorer("unknown").is_err());

    let matrix = parse_relevance_scorer_matrix(
        "smoothed-tfidf-file-stem-v2; bm25:k1=0.8,b=0.2",
    ).unwrap();
    assert_eq!(matrix.len(), 2);
    assert_eq!(matrix[0].default_label, "smoothed-tfidf-file-stem-v2");
    assert_eq!(matrix[1].default_label, "bm25-file-stem-v2-k1-0.8-b-0.2");
    assert!(parse_relevance_scorer_matrix("").is_err());
    assert!(parse_relevance_scorer_matrix(
        "bm25:k1=0.8,b=0.2;bm25:b=0.2,k1=0.8",
    ).is_err());
}

#[test]
fn relevance_specs_reject_grading_leaks_and_empty_intents() {
    let graded_candidate = serde_json::json!({
        "schemaVersion": 1,
        "corpusVersion": "candidate-test",
        "extensions": ["rs"],
        "queries": [{
            "id": "candidate-test",
            "queryClass": "test",
            "intent": "find the primary artifact",
            "request": {"terms": ["needle"], "substring": false},
            "judgments": [],
        }],
    });
    let error = serde_json::from_value::<CandidateSpec>(graded_candidate).unwrap_err();
    assert!(error.to_string().contains("unknown field `judgments`"));

    let candidate_spec = CandidateSpec {
        schema_version: 1,
        corpus_version: "candidate-test".to_string(),
        extensions: vec!["rs".to_string()],
        queries: vec![CandidateQuery {
            id: "candidate-test".to_string(),
            query_class: "test".to_string(),
            intent: "   ".to_string(),
            request: serde_json::json!({"terms": ["needle"], "substring": false}),
        }],
    };
    let panic = std::panic::catch_unwind(|| validate_candidate_spec(&candidate_spec))
        .unwrap_err();
    assert!(panic_message(panic).contains("has an empty intent"));

    let temp = tempfile::tempdir().unwrap();
    let root = crate::canonicalize_test_root(temp.path());
    fs::write(root.join("primary.rs"), "needle\n").unwrap();
    let relevance_spec = RelevanceSpec {
        schema_version: 1,
        corpus_version: "relevance-test".to_string(),
        extensions: vec!["rs".to_string()],
        global_negatives: Vec::new(),
        queries: vec![RelevanceQuery {
            id: "relevance-test".to_string(),
            query_class: "test".to_string(),
            intent: "   ".to_string(),
            request: serde_json::json!({"terms": ["needle"], "substring": false}),
            judgments: vec![Judgment {
                path: "primary.rs".to_string(),
                grade: 3,
                reason: "primary".to_string(),
            }],
            negatives: Vec::new(),
        }],
    };
    let panic = std::panic::catch_unwind(|| validate_spec(&relevance_spec, &root))
        .unwrap_err();
    assert!(panic_message(panic).contains("has an empty intent"));
}

fn write_candidate_report(output_path: &Path, report: &CandidateReport) {
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent).expect("candidate output directory should be creatable");
    }
    let mut report_json = serde_json::to_string_pretty(report).unwrap();
    report_json.push('\n');
    fs::write(output_path, report_json).expect("candidate report should be writable");
}


#[test]
#[ignore = "offline candidate collection; run explicitly before grading"]
fn write_relevance_candidates() {
    let spec_path = std::env::var_os("XRAY_RELEVANCE_CANDIDATE_SPEC")
        .map(PathBuf::from)
        .expect("XRAY_RELEVANCE_CANDIDATE_SPEC must be set");
    let corpus_root = std::env::var_os("XRAY_RELEVANCE_CORPUS")
        .map(PathBuf::from)
        .expect("XRAY_RELEVANCE_CORPUS must be set");
    let output_path = std::env::var_os("XRAY_RELEVANCE_CANDIDATES")
        .map(PathBuf::from)
        .expect("XRAY_RELEVANCE_CANDIDATES must be set");
    let output_path = validate_report_output_path(&output_path)
        .unwrap_or_else(|error| panic!("invalid candidate output path: {error}"));
    let content = fs::read_to_string(&spec_path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", spec_path.display()));
    let spec: CandidateSpec = serde_json::from_str(&content)
        .unwrap_or_else(|error| panic!("failed to parse {}: {error}", spec_path.display()));
    let scorer = relevance_scorer_from_env();
    let model = relevance_model_from_env(&scorer);
    let report = collect_candidates(
        &spec,
        &corpus_root,
        &model,
        scorer.scoring_policy,
    );
    write_candidate_report(&output_path, &report);
    println!("{}", output_path.display());
    println!("candidateDigest={}", report.candidate_digest);
    println!("corpusDigest={}", report.corpus_digest);
}

#[test]
#[ignore = "offline candidate matrix; run explicitly before grading"]
fn write_relevance_candidate_matrix() {
    let spec_path = std::env::var_os("XRAY_RELEVANCE_CANDIDATE_SPEC")
        .map(PathBuf::from)
        .expect("XRAY_RELEVANCE_CANDIDATE_SPEC must be set");
    let corpus_root = std::env::var_os("XRAY_RELEVANCE_CORPUS")
        .map(PathBuf::from)
        .expect("XRAY_RELEVANCE_CORPUS must be set");
    let output_dir = std::env::var_os("XRAY_RELEVANCE_CANDIDATE_DIR")
        .map(PathBuf::from)
        .expect("XRAY_RELEVANCE_CANDIDATE_DIR must be set");
    let raw_scorers = std::env::var("XRAY_RELEVANCE_SCORERS")
        .expect("XRAY_RELEVANCE_SCORERS must be set and valid Unicode");
    let scorers = parse_relevance_scorer_matrix(&raw_scorers)
        .unwrap_or_else(|error| panic!("invalid scorer matrix: {error}"));
    let outputs: Vec<PathBuf> = scorers.iter().map(|scorer| {
        validate_report_output_path(
            &output_dir.join(format!("{}.json", scorer.default_label)),
        ).unwrap_or_else(|error| panic!("invalid candidate output path: {error}"))
    }).collect();
    let content = fs::read_to_string(&spec_path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", spec_path.display()));
    let spec: CandidateSpec = serde_json::from_str(&content)
        .unwrap_or_else(|error| panic!("failed to parse {}: {error}", spec_path.display()));
    validate_candidate_spec(&spec);
    validate_model_sensitive_queries(
        scorers.iter().map(|scorer| scorer.scoring_policy),
        spec.queries.iter().map(|query| (query.id.as_str(), &query.request)),
    );
    let (context, corpus_root, _index_temp) = build_context(&spec.extensions, &corpus_root);
    let digest = corpus_digest(&corpus_root, &spec.extensions);

    for (scorer, output_path) in scorers.iter().zip(outputs) {
        let report = collect_candidates_with_context(
            &spec,
            &context,
            &corpus_root,
            &scorer.default_label,
            scorer.scoring_policy,
            &digest,
        );
        write_candidate_report(&output_path, &report);
        println!("{}", output_path.display());
        println!("model={}", report.model);
        println!("candidateDigest={}", report.candidate_digest);
    }
    println!("corpusDigest={digest}");
}


fn write_offline_report(output_path: &Path, report: &OfflineReport) {
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent).expect("relevance report directory should be creatable");
    }
    let mut report_json = serde_json::to_string_pretty(report).unwrap();
    report_json.push('\n');
    fs::write(output_path, report_json).expect("relevance report should be writable");
}

#[test]
#[ignore = "offline quality matrix; run explicitly after union grading"]
fn write_relevance_quality_matrix() {
    let spec_path = std::env::var_os("XRAY_RELEVANCE_SPEC")
        .map(PathBuf::from)
        .expect("XRAY_RELEVANCE_SPEC must be set");
    let corpus_root = std::env::var_os("XRAY_RELEVANCE_CORPUS")
        .map(PathBuf::from)
        .expect("XRAY_RELEVANCE_CORPUS must be set");
    let output_dir = std::env::var_os("XRAY_RELEVANCE_REPORT_DIR")
        .map(PathBuf::from)
        .expect("XRAY_RELEVANCE_REPORT_DIR must be set");
    let raw_scorers = std::env::var("XRAY_RELEVANCE_SCORERS")
        .expect("XRAY_RELEVANCE_SCORERS must be set and valid Unicode");
    let scorers = parse_relevance_scorer_matrix(&raw_scorers)
        .unwrap_or_else(|error| panic!("invalid scorer matrix: {error}"));
    let outputs: Vec<PathBuf> = scorers.iter().map(|scorer| {
        validate_report_output_path(
            &output_dir.join(format!("{}.json", scorer.default_label)),
        ).unwrap_or_else(|error| panic!("invalid relevance report path: {error}"))
    }).collect();
    let spec = load_spec_from(&spec_path);
    validate_spec(&spec, &corpus_root);
    validate_model_sensitive_queries(
        scorers.iter().map(|scorer| scorer.scoring_policy),
        spec.queries.iter().map(|query| (query.id.as_str(), &query.request)),
    );
    let (context, corpus_root, _index_temp) = build_context(&spec.extensions, &corpus_root);
    let digest = corpus_digest(&corpus_root, &spec.extensions);

    for (scorer, output_path) in scorers.iter().zip(outputs) {
        let report = run_evaluation_with_context(
            &spec,
            &context,
            &corpus_root,
            true,
            &scorer.default_label,
            scorer.scoring_policy,
            &digest,
        );
        write_offline_report(&output_path, &report);
        println!("{}", output_path.display());
        println!("model={}", report.quality.model);
        println!("metrics={}", serde_json::to_string(&report.quality.metrics).unwrap());
        println!("latency={}", serde_json::to_string(&report.latency).unwrap());
    }
    println!("corpusDigest={digest}");
}


#[test]
#[ignore = "offline quality/latency report; run explicitly before ranking changes"]
fn write_tfidf_relevance_report() {
    let spec_override = std::env::var_os("XRAY_RELEVANCE_SPEC").map(PathBuf::from);
    let corpus_override = std::env::var_os("XRAY_RELEVANCE_CORPUS").map(PathBuf::from);
    let (spec_path, corpus_root) = match (spec_override, corpus_override) {
        (Some(spec), Some(corpus)) => (spec, corpus),
        (None, None) => (fixture_root().join("queries.json"), fixture_root().join("corpus")),
        _ => panic!("XRAY_RELEVANCE_SPEC and XRAY_RELEVANCE_CORPUS must be set together"),
    };
    let output_path = std::env::var_os("XRAY_RELEVANCE_REPORT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("target/relevance/tfidf-report.json")
        });
    let output_path = validate_report_output_path(&output_path)
        .unwrap_or_else(|error| panic!("invalid relevance report path: {error}"));
    let scorer = relevance_scorer_from_env();
    let model = relevance_model_from_env(&scorer);
    let report = run_evaluation(
        &load_spec_from(&spec_path),
        &corpus_root,
        true,
        &model,
        scorer.scoring_policy,
    );
    write_offline_report(&output_path, &report);
    let baseline_candidate_name = format!("{}-baseline-candidate.json", scorer.default_label);
    let baseline_candidate_path = validate_report_output_path(
        &output_path.with_file_name(baseline_candidate_name)
    ).unwrap_or_else(|error| panic!("invalid baseline candidate path: {error}"));
    let baseline_candidate = aggregate_baseline(&report.quality);
    let mut baseline_json = serde_json::to_string_pretty(&baseline_candidate).unwrap();
    baseline_json.push('\n');
    fs::write(&baseline_candidate_path, baseline_json)
        .expect("baseline candidate should be writable");
    println!("{}", output_path.display());
    println!("{}", baseline_candidate_path.display());
    println!("{}", serde_json::to_string_pretty(&report.quality.metrics).unwrap());
    println!("{}", serde_json::to_string_pretty(&report.quality.scored_metrics).unwrap());
    println!("{}", serde_json::to_string_pretty(&report.latency).unwrap());
}
