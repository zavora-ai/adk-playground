use adk_rust::prelude::*;
use adk_rust::session::{SessionService, CreateRequest};
use adk_rust::futures::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key = std::env::var("GOOGLE_API_KEY")?;
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-3.1-flash-lite-preview")?);

    // Specialist agents
    let tech_agent: Arc<dyn Agent> = Arc::new(
        LlmAgentBuilder::new("tech_expert")
            .instruction(
                "You are a senior software engineer. Answer with code examples, \
                 technical depth, and best practices. Be precise."
            )
            .model(model.clone())
            .build()?
    );

    let general_agent: Arc<dyn Agent> = Arc::new(
        LlmAgentBuilder::new("general_helper")
            .instruction(
                "You are a friendly general assistant. Explain things simply \
                 without jargon. Use analogies. Be warm and conversational."
            )
            .model(model.clone())
            .build()?
    );

    let creative_agent: Arc<dyn Agent> = Arc::new(
        LlmAgentBuilder::new("creative_writer")
            .instruction(
                "You are a creative writer. Be imaginative, expressive, and \
                 engaging. Use vivid language and storytelling techniques."
            )
            .model(model.clone())
            .build()?
    );

    // LLM-based conditional router — the LLM classifies the query
    let router = Arc::new(
        LlmConditionalAgent::builder("smart_router", model)
            .instruction(
                "Classify the user's question as exactly ONE of: \
                 'technical' (coding, debugging, architecture), \
                 'general' (facts, knowledge, how-to), \
                 'creative' (writing, stories, brainstorming). \
                 Respond with ONLY the category name."
            )
            .route("technical", tech_agent)
            .route("general", general_agent.clone())
            .route("creative", creative_agent)
            .default_route(general_agent)
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
        agent: router,
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

    println!("=== LLM Conditional Router ===");
    println!("Routes: technical | general | creative\n");

    let message = Content::new("user")
        .with_text("Write me a short poem about a Rust programmer who finally defeated the borrow checker");
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
