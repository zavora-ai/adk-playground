use adk_core::{Content, Part, SessionId, UserId};
use adk_model::deepseek::{DeepSeekClient, DeepSeekConfig, ReasoningEffort, ThinkingMode};
use adk_rust::futures::StreamExt;
use adk_rust::prelude::*;
use adk_rust::session::{CreateRequest, SessionService};
use std::collections::HashMap;
use std::sync::Arc;

// ── DeepSeek V4 — ThinkingMode & ReasoningEffort ──
// Demonstrates the DeepSeek V4 provider features from v0.7:
//
// 1. `DeepSeekClient::v4_flash()` — fast inference, no thinking
// 2. `DeepSeekConfig::v4_pro()` with `ReasoningEffort::High` — visible reasoning
// 3. `ThinkingMode::Disabled` — explicitly turn off thinking on V4 Pro
//
// DeepSeek V4 excels at math, logic, and code reasoning tasks.
// The thinking content appears as `Part::Thinking` in the event stream.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    println!("=== DeepSeek V4 — ThinkingMode & ReasoningEffort ===\n");

    let api_key =
        std::env::var("DEEPSEEK_API_KEY").expect("Set DEEPSEEK_API_KEY in your .env file");

    // ── Part 1: V4 Flash — fast, no thinking ──
    println!("── Part 1: V4 Flash (fast, no thinking) ──\n");

    let flash = Arc::new(DeepSeekClient::v4_flash(&api_key)?);
    println!("  Model: {}", flash.name());

    let sessions = Arc::new(InMemorySessionService::new());
    let uid = UserId::new("user")?;
    let sid = SessionId::new("s1")?;
    sessions
        .create(CreateRequest {
            app_name: "playground".into(),
            user_id: uid.to_string(),
            session_id: Some(sid.to_string()),
            state: HashMap::new(),
        })
        .await?;

    let agent = Arc::new(
        LlmAgentBuilder::new("flash_agent")
            .instruction("You are a concise assistant. Answer in one sentence.")
            .model(flash)
            .build()?,
    );

    let runner = Runner::new(RunnerConfig {
        app_name: "playground".into(),
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
        intra_compaction_config: None,
        intra_compaction_summarizer: None,
    })?;

    let message = Content::new("user").with_text("What is the capital of France?");
    print!("  🤖 ");
    let mut stream = runner.run(uid.clone(), sid, message).await?;
    while let Some(event) = stream.next().await {
        let event = event?;
        if let Some(content) = event.content() {
            for part in &content.parts {
                if let Some(text) = part.text() {
                    print!("{}", text);
                }
            }
        }
    }
    println!();

    // ── Part 2: V4 Pro with thinking (high effort) ──
    println!("\n── Part 2: V4 Pro + Thinking (High Effort) ──\n");

    let pro_config = DeepSeekConfig::v4_pro(&api_key)
        .with_reasoning_effort(ReasoningEffort::High);

    let pro = Arc::new(DeepSeekClient::new(pro_config)?);
    println!("  Model: {} (reasoning_effort=high)", pro.name());

    let sid2 = SessionId::new("s2")?;
    sessions
        .create(CreateRequest {
            app_name: "playground".into(),
            user_id: uid.to_string(),
            session_id: Some(sid2.to_string()),
            state: HashMap::new(),
        })
        .await?;

    let pro_agent = Arc::new(
        LlmAgentBuilder::new("pro_agent")
            .instruction("You are a reasoning assistant. Show your work briefly.")
            .model(pro)
            .build()?,
    );

    let runner2 = Runner::new(RunnerConfig {
        app_name: "playground".into(),
        agent: pro_agent,
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
        intra_compaction_config: None,
        intra_compaction_summarizer: None,
    })?;

    let message2 = Content::new("user")
        .with_text("Is 9.11 greater than 9.8? Explain your reasoning.");

    let mut stream2 = runner2.run(uid.clone(), sid2, message2).await?;
    while let Some(event) = stream2.next().await {
        let event = event?;
        if let Some(content) = event.content() {
            for part in &content.parts {
                match part {
                    Part::Thinking { thinking, .. } => {
                        print!("  💭 {}\n", &thinking[..thinking.len().min(150)]);
                    }
                    Part::Text { text } => {
                        print!("  📝 {}", text);
                    }
                    _ => {}
                }
            }
        }
    }

    // ── Part 3: V4 Pro with thinking disabled ──
    println!("\n\n── Part 3: V4 Pro with Thinking Disabled ──\n");

    let no_think_config = DeepSeekConfig::v4_pro(&api_key)
        .with_thinking_mode(ThinkingMode::Disabled);

    let no_think = Arc::new(DeepSeekClient::new(no_think_config)?);
    println!("  Model: {} (thinking=disabled)", no_think.name());

    let sid3 = SessionId::new("s3")?;
    sessions
        .create(CreateRequest {
            app_name: "playground".into(),
            user_id: uid.to_string(),
            session_id: Some(sid3.to_string()),
            state: HashMap::new(),
        })
        .await?;

    let no_think_agent = Arc::new(
        LlmAgentBuilder::new("no_think_agent")
            .instruction("You are a concise assistant. Answer directly.")
            .model(no_think)
            .build()?,
    );

    let runner3 = Runner::new(RunnerConfig {
        app_name: "playground".into(),
        agent: no_think_agent,
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
        intra_compaction_config: None,
        intra_compaction_summarizer: None,
    })?;

    let message3 = Content::new("user").with_text("What is the speed of light in km/s?");
    print!("  🤖 ");
    let mut stream3 = runner3.run(uid, sid3, message3).await?;
    while let Some(event) = stream3.next().await {
        let event = event?;
        if let Some(content) = event.content() {
            for part in &content.parts {
                if let Some(text) = part.text() {
                    print!("{}", text);
                }
            }
        }
    }

    println!("\n\n=== Key Features ===");
    println!("• DeepSeekClient::v4_flash() — fast inference, no thinking");
    println!("• DeepSeekConfig::v4_pro() — powerful model with optional reasoning");
    println!("• ReasoningEffort::High / Max — control reasoning depth");
    println!("• ThinkingMode::Disabled — explicitly turn off thinking");
    println!("• Part::Thinking — visible chain-of-thought in event stream");
    println!("• Ideal for math, logic, code analysis, and complex reasoning");
    Ok(())
}
