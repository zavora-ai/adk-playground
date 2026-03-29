//! Skill Discovery — inject agentskills.io skills into agent prompts
//!
//! Discovers .skill.md files, scores them against a query, injects the
//! best-matching skill into the agent's system prompt, then runs the
//! skill-augmented agent against a real LLM.

use adk_core::{SessionId, UserId};
use adk_rust::futures::StreamExt;
use adk_rust::prelude::*;
use adk_rust::session::{CreateRequest, SessionService};
use adk_skill::{discover_skill_files, load_skill_index, select_skills, SelectionPolicy};
use std::collections::HashMap;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    println!("=== Skill Discovery & Agent Injection ===\n");

    // ── 1. Create skill files in a temp workspace ──
    let tmp = tempfile::tempdir()?;
    let root = tmp.path();
    let skills_dir = root.join(".skills");
    std::fs::create_dir_all(&skills_dir)?;

    std::fs::write(
        skills_dir.join("rust-expert.skill.md"),
        r#"---
name: rust-expert
description: Deep expertise in Rust programming, ownership, lifetimes, and async.
tags:
  - rust
  - programming
  - async
  - ownership
allowed-tools: []
---
You are a Rust expert. When answering questions:
1. Explain ownership and borrowing concepts clearly
2. Show idiomatic Rust patterns with code examples
3. Highlight common pitfalls and how to avoid them
4. Reference the Rust Book or std docs when helpful
"#,
    )?;

    std::fs::write(
        skills_dir.join("sql-analyst.skill.md"),
        r#"---
name: sql-analyst
description: Expert in SQL query optimization and database design.
tags:
  - sql
  - database
  - optimization
allowed-tools: []
---
You are a SQL analyst. For every query:
1. Explain the execution plan
2. Suggest index improvements
3. Rewrite for better performance
"#,
    )?;

    // ── 2. Discover and index skills ──
    let paths = discover_skill_files(root)?;
    println!("Discovered {} skill files", paths.len());

    let index = load_skill_index(root)?;
    println!("Loaded skill index: {} skills\n", index.len());

    // ── 3. Score skills against user query ──
    let query = "How do I handle async lifetimes in Rust?";
    let policy = SelectionPolicy {
        top_k: 1,
        min_score: 0.1,
        ..Default::default()
    };
    let matches = select_skills(&index, query, &policy);

    let skill_instruction = if let Some(best) = matches.first() {
        println!(
            "Best skill match: '{}' (score: {:.2})",
            best.skill.name, best.score
        );
        // Read the skill file body to inject into the prompt
        let skill_content = std::fs::read_to_string(&best.skill.path).unwrap_or_default();
        let body = skill_content.split("---").nth(2).unwrap_or("").trim();
        println!("Injecting skill into agent prompt...\n");
        format!(
            "[skill:{}]\n{}\n[/skill]\n\nBe concise (2-3 sentences per point).",
            best.skill.name, body
        )
    } else {
        println!("No skill matched — using default instruction\n");
        "You are a helpful assistant. Be concise.".to_string()
    };

    // ── 4. Build skill-augmented agent ──
    let api_key = std::env::var("GOOGLE_API_KEY")?;
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-3.1-flash-lite-preview")?);

    let agent = Arc::new(
        LlmAgentBuilder::new("skill_agent")
            .instruction(&skill_instruction)
            .model(model)
            .build()?,
    );

    let sessions = Arc::new(InMemorySessionService::new());
    sessions
        .create(CreateRequest {
            app_name: "playground".into(),
            user_id: "user".into(),
            session_id: Some("s1".into()),
            state: HashMap::new(),
        })
        .await?;

    let runner = Runner::new(RunnerConfig {
        app_name: "playground".into(),
        agent,
        session_service: sessions,
        artifact_service: None,
        memory_service: None,
        plugin_manager: None,
        run_config: None,
        compaction_config: None,
        context_cache_config: None,
        cache_capable: None,
        request_context: None,
        cancellation_token: None,
    })?;

    // ── 5. Run the skill-augmented agent ──
    println!("**User:** {}\n", query);
    print!("**Agent (skill-augmented):** ");

    let message = Content::new("user").with_text(query);
    let mut stream = runner
        .run(UserId::new("user")?, SessionId::new("s1")?, message)
        .await?;
    while let Some(event) = stream.next().await {
        let event = event?;
        if let Some(content) = &event.llm_response.content {
            for part in &content.parts {
                if let Some(text) = part.text() {
                    print!("{}", text);
                }
            }
        }
    }
    println!();

    Ok(())
}
