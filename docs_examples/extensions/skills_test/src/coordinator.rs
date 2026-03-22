//! Context Coordinator — validates the full skill→tool resolution pipeline
//!
//! Demonstrates: ContextCoordinator, ToolRegistry, ValidationMode,
//! ResolutionStrategy, and the "no phantom tools" guarantee.

use adk_skill::{
    ContextCoordinator, CoordinatorConfig, ResolutionStrategy, SelectionPolicy,
    ToolRegistry, ValidationMode, load_skill_index,
};
use adk_core::Tool;
use async_trait::async_trait;
use serde_json::Value;
use std::fs;
use std::sync::Arc;

// -- Mock tools for the registry --

struct MockTool {
    tool_name: String,
    tool_desc: String,
}

#[async_trait]
impl Tool for MockTool {
    fn name(&self) -> &str { &self.tool_name }
    fn description(&self) -> &str { &self.tool_desc }
    async fn execute(&self, _ctx: Arc<dyn adk_core::ToolContext>, _args: Value) -> adk_core::Result<Value> {
        Ok(Value::String(format!("{} executed", self.tool_name)))
    }
}

// -- Tool registry that maps names to mock implementations --

struct AppToolRegistry {
    tools: Vec<Arc<dyn Tool>>,
}

impl ToolRegistry for AppToolRegistry {
    fn resolve(&self, tool_name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.iter().find(|t| t.name() == tool_name).cloned()
    }
}

fn main() {
    println!("=== Context Coordinator Pipeline ===\n");

    // Set up workspace with skills
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let skills_dir = root.join(".skills");
    fs::create_dir_all(&skills_dir).unwrap();

    fs::write(
        skills_dir.join("search-expert.skill.md"),
        "---\nname: search-expert\ndescription: Semantic search specialist\ntags:\n  - search\nallowed-tools:\n  - rag_search\n  - web_search\n---\nUse rag_search for knowledge base queries and web_search for live data.",
    ).unwrap();

    fs::write(
        skills_dir.join("code-reviewer.skill.md"),
        "---\nname: code-reviewer\ndescription: Code review and security audit\ntags:\n  - code\n  - security\nallowed-tools:\n  - code_exec\n  - lint_check\n---\nReview code for bugs and security issues. Use code_exec to test fixes.",
    ).unwrap();

    fs::write(
        skills_dir.join("fallback.skill.md"),
        "---\nname: fallback\ndescription: General-purpose assistant\ntags:\n  - fallback\n  - default\n---\nYou are a helpful general assistant. Answer questions directly.",
    ).unwrap();

    let index = Arc::new(load_skill_index(root).unwrap());
    println!("✓ Loaded {} skills", index.len());

    // Build a registry with available tools
    let registry: Arc<dyn ToolRegistry> = Arc::new(AppToolRegistry {
        tools: vec![
            Arc::new(MockTool { tool_name: "rag_search".into(), tool_desc: "Search knowledge base".into() }),
            Arc::new(MockTool { tool_name: "web_search".into(), tool_desc: "Search the web".into() }),
            Arc::new(MockTool { tool_name: "code_exec".into(), tool_desc: "Execute code".into() }),
            // Note: lint_check is NOT registered — tests validation modes
        ],
    });

    // 1. Strict mode: rejects skills with missing tools
    let strict = ContextCoordinator::new(
        index.clone(), registry.clone(),
        CoordinatorConfig {
            policy: SelectionPolicy { top_k: 1, min_score: 0.1, ..Default::default() },
            validation_mode: ValidationMode::Strict,
            ..Default::default()
        },
    );

    let ctx = strict.build_context("review this code for security bugs");
    assert!(ctx.is_none(), "Strict mode should reject code-reviewer (missing lint_check)");
    println!("✓ Strict mode rejects skill with missing tool (lint_check)");

    // 2. Permissive mode: binds available tools, skips missing ones
    let permissive = ContextCoordinator::new(
        index.clone(), registry.clone(),
        CoordinatorConfig {
            policy: SelectionPolicy { top_k: 3, min_score: 0.1, ..Default::default() },
            validation_mode: ValidationMode::Permissive,
            ..Default::default()
        },
    );

    let ctx = permissive.build_context("review this code for security bugs").unwrap();
    // code-reviewer requests [code_exec, lint_check] but only code_exec is available
    assert!(ctx.active_tools.iter().any(|t| t.name() == "code_exec"));
    assert!(!ctx.active_tools.iter().any(|t| t.name() == "lint_check"));
    assert!(ctx.system_instruction.contains("[skill:code-reviewer]"));
    println!("✓ Permissive mode binds code_exec, skips missing lint_check");
    println!("  Instruction preview: {}...", &ctx.system_instruction[..80]);

    // 3. Query-based resolution: search query matches search-expert
    let ctx = permissive.build_context("find docs about async Rust patterns").unwrap();
    assert_eq!(ctx.provenance.skill.name, "search-expert");
    assert_eq!(ctx.active_tools.len(), 2); // rag_search + web_search
    println!("✓ Query 'async Rust patterns' → search-expert ({} tools)", ctx.active_tools.len());

    // 4. Name-based resolution: bypass scoring
    let ctx = permissive.build_context_by_name("fallback").unwrap();
    assert!(ctx.active_tools.is_empty()); // fallback has no allowed-tools
    assert!(ctx.system_instruction.contains("general assistant"));
    println!("✓ build_context_by_name('fallback') → no tools, general instruction");

    // 5. Resolution strategy cascade
    let ctx = permissive.resolve(&[
        ResolutionStrategy::ByName("nonexistent".into()),   // fails
        ResolutionStrategy::ByTag("fallback".into()),       // succeeds
    ]).unwrap();
    assert_eq!(ctx.provenance.skill.name, "fallback");
    println!("✓ Strategy cascade: ByName(miss) → ByTag(fallback) → resolved");

    // 6. Full cascade: name → query → tag
    let ctx = permissive.resolve(&[
        ResolutionStrategy::ByName("search-expert".into()),
        ResolutionStrategy::ByQuery("anything".into()),
        ResolutionStrategy::ByTag("default".into()),
    ]).unwrap();
    assert_eq!(ctx.provenance.skill.name, "search-expert");
    println!("✓ Full cascade: first match (ByName) wins");

    println!("\n=== All coordinator tests passed! ===");
}
