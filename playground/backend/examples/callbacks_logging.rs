use adk_rust::prelude::*;
use adk_rust::session::{SessionService, CreateRequest};
use adk_rust::futures::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key = std::env::var("GOOGLE_API_KEY")?;
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-2.5-flash")?);

    let agent = Arc::new(
        LlmAgentBuilder::new("logged_agent")
            .model(model)
            .instruction("You are a helpful assistant. Be brief and concise.")
            // Before callback — logs when agent starts processing
            .before_callback(Box::new(|ctx| {
                Box::pin(async move {
                    println!("[LOG] Agent '{}' starting", ctx.agent_name());
                    println!("[LOG] Session: {}", ctx.session_id());
                    println!("[LOG] User: {}", ctx.user_id());
                    Ok(None) // Continue normal execution
                })
            }))
            // After callback — logs when agent finishes
            .after_callback(Box::new(|ctx| {
                Box::pin(async move {
                    println!("[LOG] Agent '{}' completed", ctx.agent_name());
                    Ok(None) // Keep original result
                })
            }))
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

    println!("=== Callbacks: Logging Example ===\n");
    let message = Content::new("user")
        .with_text("What are the benefits of using callbacks in agent systems?");
    let mut stream = runner.run("user".into(), "s1".into(), message).await?;

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
