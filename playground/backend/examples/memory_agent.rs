//! Memory-Enhanced Agent — cross-session recall
//!
//! Stores conversation memories and recalls them in new sessions,
//! giving the agent persistent knowledge about the user.

use adk_rust::prelude::*;
use adk_rust::session::{SessionService, CreateRequest};
use adk_rust::futures::StreamExt;
use adk_core::{Content, UserId, SessionId};
use adk_memory::{InMemoryMemoryService, MemoryEntry, MemoryService, SearchRequest};
use adk_tool::tool;
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

static MEMORY_SVC: OnceLock<Arc<InMemoryMemoryService>> = OnceLock::new();

#[derive(Deserialize, JsonSchema)]
struct RecallArgs {
    /// What to search for in memory
    query: String,
}

/// Search long-term memory for relevant past conversations.
#[tool]
async fn recall_memory(args: RecallArgs) -> adk_tool::Result<serde_json::Value> {
    let svc = MEMORY_SVC.get().unwrap();
    match svc.search(SearchRequest {
        query: args.query.clone(),
        user_id: "user".into(),
        app_name: "playground".into(),
        limit: Some(5),
        min_score: None,
    }).await {
        Ok(resp) => {
            let memories: Vec<_> = resp.memories.iter().map(|m| {
                let text: String = m.content.parts.iter()
                    .filter_map(|p| p.text())
                    .collect();
                serde_json::json!({
                    "author": m.author,
                    "text": text,
                })
            }).collect();
            Ok(serde_json::json!({
                "query": args.query,
                "found": memories.len(),
                "memories": memories,
            }))
        }
        Err(e) => Ok(serde_json::json!({ "error": e.to_string() })),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    println!("=== Memory-Enhanced Agent ===\n");

    let memory_svc = Arc::new(InMemoryMemoryService::new());

    // Pre-populate with "past session" memories
    memory_svc.add_session("playground", "user", "old-session-1", vec![
        MemoryEntry {
            content: Content::new("user").with_text("I'm building a web scraper in Rust using reqwest and tokio"),
            author: "user".into(),
            timestamp: chrono::Utc::now(),
        },
        MemoryEntry {
            content: Content::new("assistant").with_text("Great choice! reqwest with tokio gives you async HTTP. Consider using select! for concurrent requests."),
            author: "assistant".into(),
            timestamp: chrono::Utc::now(),
        },
    ]).await?;

    memory_svc.add_session("playground", "user", "old-session-2", vec![
        MemoryEntry {
            content: Content::new("user").with_text("My favorite language is Rust and I prefer async/await over threads"),
            author: "user".into(),
            timestamp: chrono::Utc::now(),
        },
        MemoryEntry {
            content: Content::new("assistant").with_text("Noted! async/await with tokio is indeed more ergonomic for I/O-bound work."),
            author: "assistant".into(),
            timestamp: chrono::Utc::now(),
        },
    ]).await?;
    println!("✓ Pre-loaded 2 past sessions into memory\n");

    let _ = MEMORY_SVC.set(memory_svc.clone());

    let api_key = std::env::var("GOOGLE_API_KEY")?;
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-3.1-flash-lite-preview")?);

    let agent = Arc::new(
        LlmAgentBuilder::new("memory_agent")
            .instruction(
                "You are a helpful coding assistant with long-term memory.\n\
                 Use the recall_memory tool to search past conversations before answering.\n\
                 This helps you remember the user's preferences, projects, and past discussions.\n\
                 Always check memory first, then incorporate what you find into your response.\n\
                 If memory has relevant context, mention it naturally (e.g. 'I remember you mentioned...')."
            )
            .model(model)
            .tool(Arc::new(RecallMemory))
            .build()?
    );

    let sessions = Arc::new(InMemorySessionService::new());
    sessions.create(CreateRequest {
        app_name: "playground".into(),
        user_id: "user".into(),
        session_id: Some("s1".into()),
        state: HashMap::new(),
    }).await?;

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

    let query = "I need help with error handling in my project. What approach would suit me best?";
    println!("**User:** {}\n", query);
    print!("**Agent:** ");

    let message = Content::new("user").with_text(query);
    let mut stream = runner.run(UserId::new("user")?, SessionId::new("s1")?, message).await?;
    while let Some(event) = stream.next().await {
        let event = event?;
        if let Some(content) = &event.llm_response.content {
            for part in &content.parts {
                if let Some(text) = part.text() { print!("{}", text); }
            }
        }
    }
    println!();

    Ok(())
}
