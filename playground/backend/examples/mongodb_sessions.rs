// MongoDB Sessions — Schema-flexible document session persistence
//
// Connects to a real MongoDB instance via `MongoSessionService`.
// Demonstrates: auto-index creation, nested document state,
// schema-free evolution, CRUD operations, and agent memory.
//
// Requires: MONGODB_URL (+ GOOGLE_API_KEY for agent demo)

use adk_rust::prelude::*;
use adk_rust::session::{
    MongoSessionService, SessionService, CreateRequest, GetRequest, ListRequest, DeleteRequest,
};
use adk_rust::futures::StreamExt;
use adk_core::{UserId, SessionId};
use std::collections::HashMap;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    let db_url = std::env::var("MONGODB_URL")
        .unwrap_or_else(|_| "mongodb://adk:adk_playground@localhost:27017/?retryWrites=false".into());

    println!("# MongoDB Session Backend\n");
    println!("Connecting to `{}`...\n", db_url);

    // ── Connect & migrate ──
    let service = MongoSessionService::new(&db_url, "adk_playground").await?;
    service.migrate().await?;
    println!("✅ Connected and indexes created\n");

    // ── Create sessions with rich nested state ──
    // MongoDB shines with deeply nested, schema-flexible documents
    println!("## Creating Sessions with Nested State\n");

    let mut state1 = HashMap::new();
    state1.insert("app:version".to_string(), serde_json::json!("2.1.0"));
    state1.insert("user:profile".to_string(), serde_json::json!({
        "name": "Charlie",
        "tags": ["power-user", "early-adopter"],
        "preferences": {"theme": "dark", "language": "en"}
    }));
    state1.insert("cart_items".to_string(), serde_json::json!([]));
    state1.insert("browsing_history".to_string(), serde_json::json!([]));

    let session1 = service.create(CreateRequest {
        app_name: "mongo-demo".into(),
        user_id: "charlie".into(),
        session_id: Some("mongo-session-1".into()),
        state: state1,
    }).await?;
    println!("✅ Session 1: `{}`", session1.id());
    println!("   Nested profile, arrays (cart_items, browsing_history)");

    let mut state2 = HashMap::new();
    state2.insert("app:version".to_string(), serde_json::json!("2.1.0"));
    state2.insert("user:profile".to_string(), serde_json::json!({
        "name": "Diana",
        "tags": ["new-user"],
        "preferences": {"theme": "light", "language": "fr"}
    }));

    let session2 = service.create(CreateRequest {
        app_name: "mongo-demo".into(),
        user_id: "diana".into(),
        session_id: Some("mongo-session-2".into()),
        state: state2,
    }).await?;
    println!("✅ Session 2: `{}`\n", session2.id());

    // ── List sessions ──
    println!("## Multi-User Queries\n");
    let charlie_sessions = service.list(ListRequest {
        app_name: "mongo-demo".into(),
        user_id: "charlie".into(),
        limit: None,
        offset: None,
    }).await?;
    let diana_sessions = service.list(ListRequest {
        app_name: "mongo-demo".into(),
        user_id: "diana".into(),
        limit: None,
        offset: None,
    }).await?;
    println!("Charlie: {} session(s)", charlie_sessions.len());
    println!("Diana:   {} session(s)\n", diana_sessions.len());

    // ── Run agent with MongoDB-backed sessions ──
    println!("## Agent with MongoDB Memory\n");

    let api_key = std::env::var("GOOGLE_API_KEY")?;
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-3.1-flash-lite-preview")?);

    let agent = Arc::new(
        LlmAgentBuilder::new("mongo_agent")
            .instruction(
                "You are a shopping assistant. The customer's profile is in session state. \
                 Be friendly and concise (1-2 sentences)."
            )
            .model(model)
            .build()?
    );

    let sessions_arc: Arc<dyn SessionService> = Arc::new(service);

    let runner = Runner::new(RunnerConfig {
        app_name: "mongo-demo".into(),
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

    let msg = Content::new("user").with_text(
        "I'm looking for a birthday gift for someone who loves cooking. Budget around $50-80."
    );
    print!("**User:** Birthday gift for a cooking lover, $50-80 budget.\n\n**Agent:** ");
    let mut stream = runner.run(UserId::new("charlie")?, SessionId::new("mongo-session-1")?, msg).await?;
    while let Some(event) = stream.next().await {
        let event = event?;
        if let Some(content) = &event.llm_response.content {
            for part in &content.parts {
                if let Some(text) = part.text() { print!("{}", text); }
            }
        }
    }
    println!("\n");

    // ── Inspect stored document ──
    println!("## Document Inspection\n");
    let retrieved = sessions_arc.get(GetRequest {
        app_name: "mongo-demo".into(),
        user_id: "charlie".into(),
        session_id: "mongo-session-1".into(),
        num_recent_events: None,
        after: None,
    }).await?;
    println!("Session `{}` — {} events persisted", retrieved.id(), retrieved.events().len());
    let state = retrieved.state().all();
    if let Some(profile) = state.get("user:profile") {
        println!("Profile: {}", serde_json::to_string_pretty(profile).unwrap_or_default());
    }
    println!();

    // ── Cleanup ──
    println!("## Cleanup\n");
    sessions_arc.delete(DeleteRequest {
        app_name: "mongo-demo".into(),
        user_id: "charlie".into(),
        session_id: "mongo-session-1".into(),
    }).await?;
    sessions_arc.delete(DeleteRequest {
        app_name: "mongo-demo".into(),
        user_id: "diana".into(),
        session_id: "mongo-session-2".into(),
    }).await?;
    println!("✅ All test sessions deleted\n");

    println!("---\n");
    println!("**MongoDB strengths:** Schema flexibility, nested documents, arrays as first-class citizens, horizontal scaling, TTL indexes.");

    Ok(())
}
