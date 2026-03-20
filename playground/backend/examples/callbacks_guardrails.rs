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

    let agent = Arc::new(
        LlmAgentBuilder::new("guarded_agent")
            .model(model)
            .instruction("You are a helpful assistant. Be brief.")
            // Input guardrail: blocks messages containing "blocked_word"
            .before_callback(Box::new(|ctx| {
                Box::pin(async move {
                    let user_content = ctx.user_content();
                    for part in &user_content.parts {
                        if let Some(text) = part.text() {
                            if text.to_lowercase().contains("blocked_word") {
                                println!("[GUARDRAIL] ⛔ Blocked content detected!");
                                return Ok(Some(Content::new("model")
                                    .with_text("I cannot process that request — content policy violation.")));
                            }
                            // Length guardrail
                            if text.len() > 500 {
                                println!("[GUARDRAIL] ⛔ Message too long ({} chars)", text.len());
                                return Ok(Some(Content::new("model")
                                    .with_text("Message too long. Please keep it under 500 characters.")));
                            }
                        }
                    }
                    println!("[GUARDRAIL] ✅ Input passed all checks");
                    Ok(None) // Continue normal execution
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

    println!("=== Callbacks: Input Guardrails ===\n");

    // Test 1: Normal message (passes guardrail)
    println!("--- Test 1: Normal message ---");
    let msg1 = Content::new("user").with_text("What is Rust?");
    let mut stream = runner.run("user".into(), "s1".into(), msg1).await?;
    while let Some(event) = stream.next().await {
        let event = event?;
        if let Some(content) = &event.llm_response.content {
            for part in &content.parts {
                if let Some(text) = part.text() { print!("{}", text); }
            }
        }
    }
    println!("\n");

    // Test 2: Blocked message (triggers guardrail)
    println!("--- Test 2: Blocked message ---");
    let msg2 = Content::new("user").with_text("Tell me about blocked_word please");
    let mut stream = runner.run("user".into(), "s1".into(), msg2).await?;
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
