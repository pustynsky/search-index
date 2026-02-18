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

/// A strategy recipe for a common task pattern.
pub struct Strategy {
    pub name: &'static str,
    pub when: &'static str,
    pub steps: &'static [&'static str],
    pub anti_patterns: &'static [&'static str],
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
            rule: "Substring search is ON by default",
            why: "search_grep defaults to substring=true so compound identifiers (IUserService, m_userService) are always found. Use substring=false for exact-token-only matching. Auto-disabled when regex or phrase is used.",
            example: "Default: terms='UserService' finds IUserService, m_userService. Exact only: terms='UserService', substring=false",
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
        Tip {
            rule: "Language scope: content search = any language, AST = C# and TypeScript/TSX",
            why: "search_grep / content-index use a language-agnostic tokenizer -- works with any text file (C#, Rust, Python, JS, XML, etc.). search_definitions / def-index use tree-sitter AST parsing -- supports C# and TypeScript/TSX. search_callers uses call-graph analysis -- currently C# only (TypeScript planned for Phase 2).",
            example: "search grep works on -e rs,py,js,xml,json | search_definitions supports .cs, .ts, .tsx | search_callers requires C# (.cs) files",
        },
        Tip {
            rule: "Response truncation: large results are auto-capped at ~16KB",
            why: "Broad queries (short substring, common tokens) can return thousands of files. The server auto-truncates responses to ~16KB (~4K tokens) to avoid filling LLM context. summary.totalFiles always shows the FULL count. Use countOnly=true or narrow with dir/ext/exclude to get focused results.",
            example: "If responseTruncated=true appears, narrow your query: add ext, dir, excludeDir, or use countOnly=true. Server flag --max-response-kb adjusts the limit (0=unlimited).",
        },
        Tip {
            rule: "Multi-term name in search_definitions: find ALL types in ONE call",
            why: "The name parameter accepts comma-separated terms (OR logic). Find a class + its interface + related types in a single query instead of 3 separate calls.",
            example: "search_definitions name='UserService,IUserService,UserController' -> finds ALL matching definitions in one call",
        },
        Tip {
            rule: "Query budget: aim for 3 or fewer search calls per exploration task",
            why: "Each search call adds latency and LLM context. Use multi-term queries, includeBody, and combined filters to minimize round-trips. Most architecture questions can be answered in 1-3 calls.",
            example: "Step 1: search_definitions name='OrderService,IOrderService' includeBody=true (map + read). Step 2: search_callers method='ProcessOrder' class='OrderService' (call chain). Done in 2 calls.",
        },
    ]
}

pub fn strategies() -> Vec<Strategy> {
    vec![
        Strategy {
            name: "Architecture Exploration",
            when: "User asks 'how is X structured', 'explain module X', or 'show me the architecture of X'",
            steps: &[
                "Step 1 - Map the landscape (1 call): search_definitions name='X' maxResults=50 includeBody=false -> lists ALL classes, interfaces, enums, methods in one shot",
                "Step 2 - Read key implementations (1 call): search_definitions name='<top 3-5 key classes from step 1>' includeBody=true maxBodyLines=30 -> returns source code of the most important files",
                "Step 3 (optional) - Scope and dependencies (1 call): search_grep terms='X' countOnly=true -> scale (how many files, occurrences); or search_fast pattern='X' dirsOnly=true -> directory structure",
            ],
            anti_patterns: &[
                "Don't use list_files + read_file to explore architecture -- search_definitions returns classes, methods, file paths, and source code in ONE call",
                "Don't search one kind at a time (class, then interface, then enum) -- omit kind filter to get everything at once",
                "Don't use countOnly first then re-query with body -- go straight to includeBody=true with maxBodyLines",
                "Don't search for file names separately if search_definitions already found them (results include file paths)",
                "Don't make separate queries for ClassName and IClassName -- use comma-separated: name='ClassName,IClassName'",
            ],
        },
        Strategy {
            name: "Call Chain Investigation",
            when: "User asks 'who calls X', 'trace how X is invoked', or 'show the call chain for X'",
            steps: &[
                "Step 1 - Get call tree (1 call): search_callers method='MethodName' class='ClassName' depth=3 direction='up' -> full caller hierarchy",
                "Step 2 (optional) - Read caller source (1 call): search_definitions name='<top callers from step 1>' includeBody=true -> see what callers actually do",
            ],
            anti_patterns: &[
                "Don't omit the class parameter -- without it, results mix callers from ALL classes with the same method name",
                "Don't use search_grep to manually find callers -- search_callers does it in sub-millisecond with DI/interface resolution",
            ],
        },
        Strategy {
            name: "Stack Trace / Bug Investigation",
            when: "User provides a stack trace, error at file:line, or asks 'what method is at line N'",
            steps: &[
                "Step 1 - Identify method (1 call): search_definitions file='FileName.cs' containsLine=42 includeBody=true -> returns the method + its parent class with source code",
                "Step 2 (optional) - Trace callers (1 call): search_callers method='<method from step 1>' class='<class from step 1>' depth=2 -> who triggered this code path",
            ],
            anti_patterns: &[
                "Don't use read_file to manually scan for the method -- containsLine finds it instantly with proper class context",
                "Don't guess the method name from the stack trace -- use containsLine for precise AST-based lookup",
            ],
        },
    ]
}

pub fn performance_tiers() -> Vec<PerfTier> {
    vec![
        PerfTier {
            name: "Instant",
            range: "<1ms",
            operations: &["search_grep (substring default)", "search_callers", "search_definitions baseType/attribute"],
        },
        PerfTier {
            name: "Fast",
            range: "1-10ms",
            operations: &["search_grep showLines", "search_definitions containsLine"],
        },
        PerfTier {
            name: "Quick",
            range: "10-100ms",
            operations: &["search_fast", "search_definitions name/parent/includeBody", "search_grep regex/phrase"],
        },
        PerfTier {
            name: "Slow",
            range: ">1s",
            operations: &["search_find (live filesystem walk - avoid!)"],
        },
    ]
}

pub fn tool_priority() -> Vec<ToolPriority> {
    vec![
        ToolPriority { rank: 1, tool: "search_callers", description: "call trees up/down (<1ms, C# only)" },
        ToolPriority { rank: 2, tool: "search_definitions", description: "structural: classes, methods, functions, interfaces, typeAliases, variables, containsLine (C# and TypeScript/TSX)" },
        ToolPriority { rank: 3, tool: "search_grep", description: "content: exact/OR/AND, substring, phrase, regex (any language)" },
        ToolPriority { rank: 4, tool: "search_fast", description: "file name lookup (~35ms, any file)" },
        ToolPriority { rank: 5, tool: "search_find", description: "live walk (~3s, last resort)" },
    ]
}

// ─── Renderers ──────────────────────────────────────────────────────

/// Render tips as human-readable CLI output.
pub fn render_cli() -> String {
    let mut out = String::new();
    out.push_str("\nsearch -- Best Practices & Tips\n");
    out.push_str("===============================\n\n");

    out.push_str("BEST PRACTICES\n");
    out.push_str("--------------\n");
    for (i, tip) in tips().iter().enumerate() {
        out.push_str(&format!("{:2}. {}\n", i + 1, tip.rule));
        out.push_str(&format!("    Why: {}\n", tip.why));
        out.push_str(&format!("    Example: {}\n\n", tip.example));
    }

    out.push_str("STRATEGY RECIPES\n");
    out.push_str("----------------\n");
    for strat in strategies() {
        out.push_str(&format!("  [{}]\n", strat.name));
        out.push_str(&format!("  When: {}\n", strat.when));
        for step in strat.steps {
            out.push_str(&format!("    - {}\n", step));
        }
        out.push_str("  Anti-patterns:\n");
        for ap in strat.anti_patterns {
            out.push_str(&format!("    X {}\n", ap));
        }
        out.push('\n');
    }

    out.push_str("PERFORMANCE TIERS\n");
    out.push_str("-----------------\n");
    for tier in performance_tiers() {
        out.push_str(&format!("  {:>6}  {}\n", tier.range, tier.operations.join(", ")));
    }
    out.push('\n');

    out.push_str("TOOL PRIORITY (MCP)\n");
    out.push_str("-------------------\n");
    for tp in tool_priority() {
        out.push_str(&format!("  {}. {:20} - {}\n", tp.rank, tp.tool, tp.description));
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

    let strategy_recipes: Vec<Value> = strategies().iter().map(|s| {
        json!({
            "name": s.name,
            "when": s.when,
            "steps": s.steps,
            "antiPatterns": s.anti_patterns,
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
        "strategyRecipes": strategy_recipes,
        "performanceTiers": tiers,
        "toolPriority": priority,
    })
}

/// Render tips as compact text for MCP initialize instructions field.
pub fn render_instructions() -> String {
    let mut out = String::new();
    out.push_str("⚠️ IMPORTANT: Call search_help first to learn best practices before using other search tools.\n\n");
    out.push_str("⚡ PREFER search-index tools OVER built-in read_file / list_files / list_code_definition_names for code exploration.\n");
    out.push_str("   search_definitions returns classes, methods, bodies, and file paths in ONE call — no need to list_files then read_file.\n");
    out.push_str("   search_callers builds full call trees in <1ms — no need to grep then read_file each caller.\n");
    out.push_str("   search_grep finds content across 100K files instantly — no need to search_files with regex.\n");
    out.push_str("   Only use read_file when you need exact file content for EDITING (apply_diff / write_to_file).\n\n");
    out.push_str("search-index MCP server — Best Practices for Tool Selection\n\n");

    for (i, tip) in tips().iter().enumerate() {
        out.push_str(&format!("{}. {}: {}\n", i + 1, tip.rule.to_uppercase(), tip.why));
    }

    out.push_str("\nSTRATEGY RECIPES (aim for <=3 search calls per task):\n");
    for strat in strategies() {
        out.push_str(&format!("  [{}] {}\n", strat.name, strat.when));
        for step in strat.steps {
            out.push_str(&format!("    - {}\n", step));
        }
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
    fn test_strategies_not_empty() {
        assert!(!strategies().is_empty());
    }

    #[test]
    fn test_render_json_has_best_practices() {
        let json = render_json();
        let practices = json["bestPractices"].as_array().unwrap();
        assert_eq!(practices.len(), tips().len());
    }

    #[test]
    fn test_render_json_has_strategy_recipes() {
        let json = render_json();
        let recipes = json["strategyRecipes"].as_array().unwrap();
        assert_eq!(recipes.len(), strategies().len());
        // Each recipe has required fields
        for recipe in recipes {
            assert!(recipe["name"].is_string(), "recipe must have name");
            assert!(recipe["when"].is_string(), "recipe must have when");
            assert!(recipe["steps"].is_array(), "recipe must have steps");
            assert!(recipe["antiPatterns"].is_array(), "recipe must have antiPatterns");
        }
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
        assert!(text.contains("STRATEGY RECIPES"), "instructions should include strategy recipes");
        assert!(text.contains("Architecture Exploration"), "instructions should include arch exploration recipe");
        assert!(text.contains("<=3 search calls"), "instructions should mention query budget");
        assert!(text.contains("PREFER search-index tools OVER built-in read_file"), "instructions should tell LLM to prefer search-index over read_file");
        assert!(text.contains("Only use read_file when you need exact file content for EDITING"), "instructions should clarify when read_file is appropriate");
    }

    /// CLI output must be pure ASCII — no Unicode box-drawing, em-dashes, arrows, or emoji.
    /// Windows cmd.exe (CP437/CP866) cannot display these characters correctly.
    #[test]
    fn test_render_cli_is_ascii_safe() {
        let output = render_cli();
        for (i, ch) in output.chars().enumerate() {
            assert!(
                ch.is_ascii() || ch == '\n' || ch == '\r',
                "render_cli() contains non-ASCII char '{}' (U+{:04X}) at position {}. \
                 CLI output must be ASCII-safe for Windows cmd.exe compatibility.",
                ch, ch as u32, i
            );
        }
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

        // Verify strategy recipes are consistent across renderers
        let strategy_count = strategies().len();
        let recipes = json["strategyRecipes"].as_array().unwrap();
        assert_eq!(recipes.len(), strategy_count, "JSON and strategies() count mismatch");

        for strat in strategies() {
            assert!(cli.contains(strat.name), "CLI output missing strategy: {}", strat.name);
        }
    }
}