//! Single source of truth for best practices and tips.
//! Used by: CLI `search tips`, MCP `search_help` tool, MCP `instructions` field.

use serde_json::{json, Value};

/// A single best practice tip.
pub struct Tip {
    pub rule: &'static str,
    pub why: &'static str,
    pub example: &'static str,
}

/// Performance tier description.
pub struct PerfTier {
    pub name: &'static str,
    pub range: &'static str,
    pub operations: &'static [&'static str],
}

/// Tool priority entry.
pub struct ToolPriority {
    pub rank: u8,
    pub tool: &'static str,
    pub description: &'static str,
}

// ─── Single source of truth ─────────────────────────────────────────

pub fn tips() -> Vec<Tip> {
    vec![
        Tip {
            rule: "File lookup: use search_fast, not search_find",
            why: "search_fast uses a pre-built index (~35ms). search_find does a live filesystem walk (~3s). 90x+ faster.",
            example: "search_fast with pattern='UserService' instead of search_find",
        },
        Tip {
            rule: "Multi-term OR: find all variants in ONE query",
            why: "Comma-separated terms with mode='or' finds files containing ANY term. Much faster than separate queries.",
            example: "search grep \"UserService,IUserService,UserServiceFactory\" -e cs  |  MCP: terms='...', mode='or'",
        },
        Tip {
            rule: "AND mode: find files containing ALL terms",
            why: "mode='and' finds files where ALL comma-separated terms co-occur. Useful for finding DI registrations.",
            example: "search grep \"ServiceProvider,IUserService\" -e cs --all  |  MCP: terms='...', mode='and'",
        },
        Tip {
            rule: "C#/Java substring search: use substring=true",
            why: "Default exact-token mode won't find 'UserService' inside 'DeleteUserServiceCacheEntry'. Trigram index, ~1ms.",
            example: "search grep \"UserService\" -e cs --substring  |  MCP: terms='UserService', substring=true",
        },
        Tip {
            rule: "Phrase search: exact multi-word match",
            why: "phrase=true finds exact adjacent word sequences. Slower (~80ms) but precise.",
            example: "search grep \"new HttpClient\" -e cs --phrase  |  MCP: terms='new HttpClient', phrase=true",
        },
        Tip {
            rule: "Regex pattern search",
            why: "Full regex for pattern matching. Also works in search_definitions name parameter.",
            example: "search grep \"I[A-Z]\\w+Cache\" -e cs --regex  |  MCP: terms='I[A-Z]\\w+Cache', regex=true",
        },
        Tip {
            rule: "Exclude test/mock dirs for production-only results",
            why: "Half the results are often test files. Use excludeDir to filter them out.",
            example: "--exclude-dir test --exclude-dir Mock  |  MCP: excludeDir=['test','Mock','UnitTests']",
        },
        Tip {
            rule: "Call chain tracing: search_callers (up and down)",
            why: "Single sub-millisecond request replaces 7+ sequential grep + read_file calls. direction='up' (callers) or 'down' (callees).",
            example: "MCP: search_callers method='GetUserAsync', class='UserService', depth=2, direction='up'",
        },
        Tip {
            rule: "Always specify class in search_callers",
            why: "Without class, results mix callers from ALL classes with same method name. Misleading call trees.",
            example: "MCP: search_callers method='ExecuteAsync', class='OrderProcessor'",
        },
        Tip {
            rule: "Stack trace analysis: containsLine",
            why: "Given file + line number, returns innermost method AND parent class. No manual read_file needed.",
            example: "MCP: search_definitions file='UserService.cs', containsLine=42",
        },
        Tip {
            rule: "Read method source: use includeBody=true",
            why: "search_definitions with includeBody=true returns method body inline, eliminating read_file round-trips. Use maxBodyLines/maxTotalBodyLines for budget.",
            example: "MCP: search_definitions parent='UserService', includeBody=true, maxBodyLines=20",
        },
        Tip {
            rule: "Body budgets: 0 means unlimited",
            why: "Default limits: 100 lines/def, 500 total. Set maxBodyLines=0, maxTotalBodyLines=0 for unlimited output.",
            example: "MCP: search_definitions parent='UserService', includeBody=true, maxBodyLines=0, maxTotalBodyLines=0",
        },
        Tip {
            rule: "Reconnaissance: use countOnly=true",
            why: "search_grep with countOnly=true returns ~46 tokens (counts only) vs 265+ for full results. Perfect for 'how many files use X?'.",
            example: "search grep \"HttpClient\" -e cs --count-only  |  MCP: terms='HttpClient', countOnly=true",
        },
        Tip {
            rule: "Search ANY indexed file type: XML, csproj, config, etc.",
            why: "search_grep works with all file extensions passed to --ext. Use ext='csproj' to find NuGet dependencies, ext='xml,config,manifestxml' for configuration values.",
            example: "search grep \"Newtonsoft.Json\" -e csproj  |  MCP: terms='Newtonsoft.Json', ext='csproj'",
        },
    ]
}

pub fn performance_tiers() -> Vec<PerfTier> {
    vec![
        PerfTier {
            name: "Instant",
            range: "<1ms",
            operations: &["search_grep exact/OR/AND", "search_callers", "search_definitions baseType/attribute"],
        },
        PerfTier {
            name: "Fast",
            range: "1-10ms",
            operations: &["search_grep substring/showLines", "search_definitions containsLine"],
        },
        PerfTier {
            name: "Quick",
            range: "10-100ms",
            operations: &["search_fast", "search_definitions name/parent/includeBody", "search_grep regex/phrase"],
        },
        PerfTier {
            name: "Slow",
            range: ">1s",
            operations: &["search_find (live filesystem walk — avoid!)"],
        },
    ]
}

pub fn tool_priority() -> Vec<ToolPriority> {
    vec![
        ToolPriority { rank: 1, tool: "search_callers", description: "call trees up/down (<1ms)" },
        ToolPriority { rank: 2, tool: "search_definitions", description: "structural: classes, methods, containsLine" },
        ToolPriority { rank: 3, tool: "search_grep", description: "content: exact/OR/AND, substring, phrase, regex" },
        ToolPriority { rank: 4, tool: "search_fast", description: "file name lookup (~35ms)" },
        ToolPriority { rank: 5, tool: "search_find", description: "live walk (~3s, last resort)" },
    ]
}

// ─── Renderers ──────────────────────────────────────────────────────

/// Render tips as human-readable CLI output.
pub fn render_cli() -> String {
    let mut out = String::new();
    out.push_str("\nsearch — Best Practices & Tips\n");
    out.push_str("═══════════════════════════════\n\n");

    out.push_str("BEST PRACTICES\n");
    out.push_str("──────────────\n");
    for (i, tip) in tips().iter().enumerate() {
        out.push_str(&format!("{:2}. {}\n", i + 1, tip.rule));
        out.push_str(&format!("    Why: {}\n", tip.why));
        out.push_str(&format!("    Example: {}\n\n", tip.example));
    }

    out.push_str("PERFORMANCE TIERS\n");
    out.push_str("─────────────────\n");
    for tier in performance_tiers() {
        out.push_str(&format!("  {:>6}  {}\n", tier.range, tier.operations.join(", ")));
    }
    out.push('\n');

    out.push_str("TOOL PRIORITY (MCP)\n");
    out.push_str("───────────────────\n");
    for tp in tool_priority() {
        out.push_str(&format!("  {}. {:20} — {}\n", tp.rank, tp.tool, tp.description));
    }
    out.push('\n');

    out
}

/// Render tips as JSON for MCP search_help tool.
pub fn render_json() -> Value {
    let best_practices: Vec<Value> = tips().iter().map(|t| {
        json!({
            "rule": t.rule,
            "why": t.why,
            "example": t.example,
        })
    }).collect();

    let mut tiers = serde_json::Map::new();
    for tier in performance_tiers() {
        let key = format!("{}_{}", tier.name.to_lowercase(), tier.range.replace(['<', '>', ' '], ""));
        tiers.insert(key, json!(tier.operations));
    }

    let priority: Vec<Value> = tool_priority().iter().map(|tp| {
        json!(format!("{}. {} — {}", tp.rank, tp.tool, tp.description))
    }).collect();

    json!({
        "bestPractices": best_practices,
        "performanceTiers": tiers,
        "toolPriority": priority,
    })
}

/// Render tips as compact text for MCP initialize instructions field.
pub fn render_instructions() -> String {
    let mut out = String::new();
    out.push_str("⚠️ IMPORTANT: Call search_help first to learn best practices before using other search tools.\n\n");
    out.push_str("search-index MCP server — Best Practices for Tool Selection\n\n");

    for (i, tip) in tips().iter().enumerate() {
        out.push_str(&format!("{}. {}: {}\n", i + 1, tip.rule.to_uppercase(), tip.why));
    }

    out.push_str("\nTOOL PRIORITY:\n");
    for tp in tool_priority() {
        out.push_str(&format!("  {}. {} — {}\n", tp.rank, tp.tool, tp.description));
    }
    out.push_str("\nCall search_help for a detailed JSON guide with examples.\n");

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tips_not_empty() {
        assert!(!tips().is_empty());
    }

    #[test]
    fn test_performance_tiers_not_empty() {
        assert!(!performance_tiers().is_empty());
    }

    #[test]
    fn test_tool_priority_not_empty() {
        assert!(!tool_priority().is_empty());
    }

    #[test]
    fn test_render_cli_contains_all_tips() {
        let output = render_cli();
        for tip in tips() {
            assert!(output.contains(tip.rule), "CLI output missing tip: {}", tip.rule);
        }
    }

    #[test]
    fn test_render_json_has_best_practices() {
        let json = render_json();
        let practices = json["bestPractices"].as_array().unwrap();
        assert_eq!(practices.len(), tips().len());
    }

    #[test]
    fn test_render_instructions_contains_key_terms() {
        let text = render_instructions();
        assert!(text.contains("search_fast"), "instructions should mention search_fast");
        assert!(text.contains("search_callers"), "instructions should mention search_callers");
        assert!(text.contains("substring"), "instructions should mention substring");
        assert!(text.contains("containsLine"), "instructions should mention containsLine");
        assert!(text.contains("includeBody"), "instructions should mention includeBody");
        assert!(text.contains("countOnly"), "instructions should mention countOnly");
        assert!(text.contains("search_help"), "instructions should mention search_help");
    }

    #[test]
    fn test_all_renderers_consistent_tip_count() {
        let tip_count = tips().len();
        let json = render_json();
        let practices = json["bestPractices"].as_array().unwrap();
        assert_eq!(practices.len(), tip_count, "JSON and tips() count mismatch");

        // Verify CLI output mentions each tip rule
        let cli = render_cli();
        for tip in tips() {
            assert!(cli.contains(tip.rule), "CLI output missing tip: {}", tip.rule);
        }
    }
}