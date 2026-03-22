use adk_rust::prelude::*;
use adk_rust::session::{SessionService, CreateRequest};
use adk_rust::futures::StreamExt;
use adk_core::{UserId, SessionId};
use std::collections::HashMap;
use std::sync::Arc;

// ── DeepSeek Reasoner with Chain-of-Thought ──
// Showcases DeepSeek's reasoning model: `DeepSeekConfig::reasoner()` enables
// thinking mode where the model shows its chain-of-thought reasoning process
// before delivering the final answer. Ideal for math, logic, and coding puzzles.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key = std::env::var("DEEPSEEK_API_KEY")
        .expect("Set DEEPSEEK_API_KEY in your .env file");

    // DeepSeek Reasoner: thinking mode with chain-of-thought
    let model = Arc::new(DeepSeekClient::new(
        DeepSeekConfig::reasoner(api_key)
            .with_max_tokens(4096)
    )?);

    let agent = Arc::new(
        LlmAgentBuilder::new("reasoning_tutor")
            .instruction(
                "You are a math and logic tutor. Show your reasoning step by step. \
                 Break complex problems into smaller parts and solve each one."
            )
            .model(model)
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

    println!("🧮 DeepSeek Reasoner — Chain-of-Thought Reasoning\n");

    let message = Content::new("user")
        .with_text(
            "A farmer has 3 fields. The first field is twice the size of the second. \
             The third field is 10 acres more than the first. Together they total 130 acres. \
             How large is each field?"
        );
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
