// PostgreSQL Sessions — ACID-compliant session persistence
//
// Connects to a real PostgreSQL instance via `PostgresSessionService`.
// Demonstrates: auto-migration, three-tier state (app/user/session),
// CRUD operations, event persistence, and multi-turn agent memory.
//
// Requires: POSTGRES_URL (+ GOOGLE_API_KEY for agent demo)

use adk_rust::prelude::*;
use adk_rust::session::{
    PostgresSessionService, SessionService, CreateRequest, GetRequest, ListRequest, DeleteRequest,
};
use adk_rust::futures::StreamExt;
use adk_core::{UserId, SessionId};
use std::collections::HashMap;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    let db_url = std::env::var("POSTGRES_URL")
        .unwrap_or_else(|_| "postgres://adk:adk_playground@localhost:5433/adk_sessions".into());

    println!("# PostgreSQL Session Backend\n");
    println!("Connecting to `{}`...\n", db_url);

    // ── Connect & migrate ──
    let service = PostgresSessionService::new(&db_url).await?;
    service.migrate().await?;
    println!("✅ Connected and migrated (sessions, events, app_states, user_states)\n");

    // ── Create sessions with three-tier state ──
    println!("## Creating Sessions\n");

    let mut state1 = HashMap::new();
    state1.insert("app:version".to_string(), serde_json::json!("2.1.0"));
    state1.insert("app:environment".to_string(), serde_json::json!("playground"));
    state1.insert("user:plan".to_string(), serde_json::json!("enterprise"));
    state1.insert("user:timezone".to_string(), serde_json::json!("America/New_York"));
    state1.insert("context".to_string(), serde_json::json!("onboarding"));

    let session1 = service.create(CreateRequest {
        app_name: "pg-demo".into(),
        user_id: "alice".into(),
        session_id: Some("pg-session-1".into()),
        state: state1,
    }).await?;
    println!("✅ Session 1: `{}`", session1.id());
    println!("   app:version=2.1.0  user:plan=enterprise  context=onboarding");

    let mut state2 = HashMap::new();
    state2.insert("app:version".to_string(), serde_json::json!("2.1.0"));
    state2.insert("user:plan".to_string(), serde_json::json!("free"));
    state2.insert("context".to_string(), serde_json::json!("support"));

    let session2 = service.create(CreateRequest {
        app_name: "pg-demo".into(),
        user_id: "bob".into(),
        session_id: Some("pg-session-2".into()),
        state: state2,
    }).await?;
    println!("✅ Session 2: `{}`\n", session2.id());

    // ── List sessions per user ──
    println!("## Listing Sessions\n");
    let alice_sessions = service.list(ListRequest {
        app_name: "pg-demo".into(),
        user_id: "alice".into(),
        limit: None,
        offset: None,
    }).await?;
    println!("Alice: {} session(s)", alice_sessions.len());

    let bob_sessions = service.list(ListRequest {
        app_name: "pg-demo".into(),
        user_id: "bob".into(),
        limit: None,
        offset: None,
    }).await?;
    println!("Bob:   {} session(s)\n", bob_sessions.len());

    // ── Run agent with PostgreSQL-backed sessions ──
    println!("## Agent with PostgreSQL Memory\n");

    let api_key = std::env::var("GOOGLE_API_KEY")?;
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-3.1-flash-lite-preview")?);

    let agent = Arc::new(
        LlmAgentBuilder::new("pg_agent")
            .instruction(
                "You are a helpful assistant. Remember context from previous messages. \
                 Be concise (1-2 sentences)."
            )
            .model(model)
            .build()?
    );

    let sessions_arc: Arc<dyn SessionService> = Arc::new(service);

    let runner = Runner::new(RunnerConfig {
        app_name: "pg-demo".into(),
        agent,
        session_service: sessions_arc.clone(),
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

    // Turn 1
    let msg1 = Content::new("user").with_text("My name is Alice and I love PostgreSQL. Remember that!");
    print!("**User:** My name is Alice and I love PostgreSQL.\n\n**Agent:** ");
    let mut stream = runner.run(UserId::new("alice")?, SessionId::new("pg-session-1")?, msg1).await?;
    while let Some(event) = stream.next().await {
        let event = event?;
        if let Some(content) = &event.llm_response.content {
            for part in &content.parts {
                if let Some(text) = part.text() { print!("{}", text); }
            }
        }
    }
    println!("\n");

    // Turn 2 — agent should recall from PostgreSQL-persisted history
    let msg2 = Content::new("user").with_text("What's my name and what database do I love?");
    print!("**User:** What's my name and what database do I love?\n\n**Agent (recall):** ");
    let mut stream2 = runner.run(UserId::new("alice")?, SessionId::new("pg-session-1")?, msg2).await?;
    while let Some(event) = stream2.next().await {
        let event = event?;
        if let Some(content) = &event.llm_response.content {
            for part in &content.parts {
                if let Some(text) = part.text() { print!("{}", text); }
            }
        }
    }
    println!("\n");

    // ── Inspect persisted state ──
    println!("## Session State Inspection\n");
    let retrieved = sessions_arc.get(GetRequest {
        app_name: "pg-demo".into(),
        user_id: "alice".into(),
        session_id: "pg-session-1".into(),
        num_recent_events: None,
        after: None,
    }).await?;
    println!("Session `{}` — {} events persisted", retrieved.id(), retrieved.events().len());
    let state_keys: Vec<_> = retrieved.state().all().keys().cloned().collect();
    println!("State keys: {:?}\n", state_keys);

    // ── Cleanup ──
    println!("## Cleanup\n");
    sessions_arc.delete(DeleteRequest {
        app_name: "pg-demo".into(),
        user_id: "alice".into(),
        session_id: "pg-session-1".into(),
    }).await?;
    sessions_arc.delete(DeleteRequest {
        app_name: "pg-demo".into(),
        user_id: "bob".into(),
        session_id: "pg-session-2".into(),
    }).await?;
    println!("✅ All test sessions deleted (CASCADE removed events too)\n");

    println!("---\n");
    println!("**PostgreSQL strengths:** ACID transactions, JSONB state queries, advisory-lock migrations, CASCADE deletes, connection pooling.");

    Ok(())
}
