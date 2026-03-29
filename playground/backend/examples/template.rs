use adk_core::{SessionId, UserId};
use adk_rust::futures::StreamExt;
use adk_rust::prelude::*;
use adk_rust::session::{CreateRequest, SessionService};
use std::collections::HashMap;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key = std::env::var("GOOGLE_API_KEY")?;
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-3.1-flash-lite-preview")?);

    // Instruction templates use {key} placeholders resolved from session state
    let agent = Arc::new(
        LlmAgentBuilder::new("personalized_assistant")
            .instruction(
                "You are assisting {user:name} who speaks {user:language}. \
                 Always respond in {user:language}. \
                 Their expertise level is {user:expertise}. \
                 Adjust your explanations accordingly.",
            )
            .model(model)
            .build()?,
    );

    let sessions = Arc::new(InMemorySessionService::new());

    // Pre-seed session state with user context
    let mut state = HashMap::new();
    state.insert("user:name".to_string(), "Alice".into());
    state.insert("user:language".to_string(), "French".into());
    state.insert("user:expertise".to_string(), "beginner".into());

    sessions
        .create(CreateRequest {
            app_name: "playground".into(),
            user_id: "user".into(),
            session_id: Some("s1".into()),
            state,
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

    // The agent will respond in French, adapted for a beginner named Alice
    let message = Content::new("user").with_text("Explain what an API is");
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
