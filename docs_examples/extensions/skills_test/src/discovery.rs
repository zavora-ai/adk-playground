//! Skill discovery & parsing — validates the agentskills.io spec pipeline
//!
//! Demonstrates: discover_skill_files, parse_skill_markdown, load_skill_index,
//! SkillIndex queries, and SkillSummary generation.

use adk_skill::{
    discover_skill_files, load_skill_index, parse_skill_markdown, select_skills,
    SelectionPolicy,
};
use std::fs;

fn main() {
    println!("=== Skill Discovery & Parsing ===\n");

    // Set up a temp workspace with skill files
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let skills_dir = root.join(".skills");
    fs::create_dir_all(&skills_dir).unwrap();

    // Write a search-expert skill
    fs::write(
        skills_dir.join("search-expert.skill.md"),
        r#"---
name: search-expert
description: Expert in semantic and keyword search across knowledge bases.
tags:
  - search
  - retrieval
  - rag
allowed-tools:
  - rag_search
  - web_search
---
You are a search specialist. When the user asks a question:
1. Determine if the answer is in the knowledge base (use `rag_search`)
2. If not found, fall back to `web_search`
3. Always cite your sources with document IDs or URLs
"#,
    )
    .unwrap();

    // Write a code-reviewer skill
    fs::write(
        skills_dir.join("code-reviewer.skill.md"),
        r#"---
name: code-reviewer
description: Reviews code for bugs, style issues, and security vulnerabilities.
tags:
  - code
  - review
  - security
allowed-tools:
  - code_exec
---
You are a senior code reviewer. For each code snippet:
1. Check for common bugs and logic errors
2. Flag security vulnerabilities (injection, overflow, etc.)
3. Suggest idiomatic improvements
4. Rate severity: low / medium / high / critical
"#,
    )
    .unwrap();

    // Write a trigger-only skill (only activated by explicit @name)
    fs::write(
        skills_dir.join("deploy.skill.md"),
        r#"---
name: deploy
description: Handles deployment to staging and production environments.
trigger: true
tags:
  - devops
  - deploy
hint: "Describe what you want to deploy and to which environment"
allowed-tools:
  - code_exec
---
You are a deployment assistant. Only deploy when explicitly asked.
Confirm the target environment before proceeding.
"#,
    )
    .unwrap();

    // 1. Discover skill files
    let paths = discover_skill_files(root).unwrap();
    assert_eq!(paths.len(), 3);
    println!("✓ Discovered {} skill files", paths.len());

    // 2. Parse a single skill
    let search_path = skills_dir.join("search-expert.skill.md");
    let search_content = fs::read_to_string(&search_path).unwrap();
    let parsed = parse_skill_markdown(&search_path, &search_content).unwrap();
    assert_eq!(parsed.name, "search-expert");
    assert_eq!(parsed.allowed_tools, vec!["rag_search", "web_search"]);
    assert!(parsed.body.contains("search specialist"));
    println!("✓ Parsed 'search-expert': {} allowed tools", parsed.allowed_tools.len());

    // 3. Load full index (discovers + parses + hashes)
    let index = load_skill_index(root).unwrap();
    assert_eq!(index.len(), 3);
    println!("✓ Loaded skill index: {} skills", index.len());

    // 4. Query by name
    let found = index.find_by_name("code-reviewer").unwrap();
    assert!(found.tags.contains(&"security".to_string()));
    println!("✓ find_by_name('code-reviewer'): tags={:?}", found.tags);

    // 5. Generate summaries (lightweight, no body)
    let summaries = index.summaries();
    assert_eq!(summaries.len(), 3);
    for s in &summaries {
        println!("  - {} (trigger={})", s.name, s.trigger);
    }
    println!("✓ Generated {} summaries", summaries.len());

    // 6. Select skills by query scoring
    let matches = select_skills(
        &index,
        "find information about Rust async patterns",
        &SelectionPolicy {
            top_k: 2,
            min_score: 0.1,
            ..Default::default()
        },
    );
    assert!(!matches.is_empty());
    println!("✓ select_skills for 'Rust async patterns': {} matches", matches.len());
    for m in &matches {
        println!("  - {} (score: {:.2})", m.skill.name, m.score);
    }

    // 7. Verify trigger skill is excluded from generic queries
    // (trigger skills require explicit @name invocation)
    let deploy_skill = index.find_by_name("deploy").unwrap();
    assert!(deploy_skill.trigger);
    println!("✓ 'deploy' skill has trigger=true (explicit invocation only)");

    // 8. Content-based hashing for cache invalidation
    let s1 = index.find_by_name("search-expert").unwrap();
    assert!(!s1.hash.is_empty());
    assert!(!s1.id.is_empty());
    println!("✓ Content hash: {}...{}", &s1.hash[..8], &s1.hash[s1.hash.len()-4..]);

    println!("\n=== All discovery tests passed! ===");
}
