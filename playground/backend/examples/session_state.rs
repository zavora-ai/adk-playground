use adk_rust::prelude::*;
use adk_rust::session::{SessionService, CreateRequest, GetRequest};
use adk_rust::futures::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key = std::env::var("GOOGLE_API_KEY")?;
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-3.1-flash-lite-preview")?);

    let agent = Arc::new(
        LlmAgentBuilder::new("stateful_assistant")
            .instruction(
                "You are a helpful assistant. Remember context from previous messages. \
                 Be concise."
            )
            .model(model)
            .build()?
    );

    // Create session service and a session with initial state
    let sessions = Arc::new(InMemorySessionService::new());
    let mut initial_state = HashMap::new();
    initial_state.insert("user_preference".to_string(), "concise answers".into());

    sessions.create(CreateRequest {
        app_name: "demo".into(),
        user_id: "alice".into(),
        session_id: Some("session-1".into()),
        state: initial_state,
    }).await?;

    let runner = Runner::new(RunnerConfig {
        app_name: "demo".into(),
        agent,
        session_service: sessions.clone(),
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
    println!("--- Turn 1 ---");
    let msg1 = Content::new("user").with_text("My name is Alice and I love Rust.");
    let mut stream = runner.run("alice".into(), "session-1".into(), msg1).await?;
    while let Some(event) = stream.next().await {
        let event = event?;
        if let Some(content) = &event.llm_response.content {
            for part in &content.parts {
                if let Some(text) = part.text() { print!("{}", text); }
            }
        }
    }
    println!("\n");

    // Turn 2 — agent should remember the name from Turn 1
    println!("--- Turn 2 ---");
    let msg2 = Content::new("user").with_text("What's my name and what language do I like?");
    let mut stream = runner.run("alice".into(), "session-1".into(), msg2).await?;
    while let Some(event) = stream.next().await {
        let event = event?;
        if let Some(content) = &event.llm_response.content {
            for part in &content.parts {
                if let Some(text) = part.text() { print!("{}", text); }
            }
        }
    }
    println!();

    // Show session info
    let session = sessions.get(GetRequest {
        app_name: "demo".into(),
        user_id: "alice".into(),
        session_id: "session-1".into(),
        num_recent_events: None,
        after: None,
    }).await?;
    println!("\nSession ID: {}", session.id());
    println!("Events count: {}", session.events().len());
    Ok(())
}
