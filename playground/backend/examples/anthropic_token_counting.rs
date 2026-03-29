use adk_core::{Content, Llm, LlmRequest, Part, SessionId, UserId};
use adk_rust::futures::StreamExt;
use adk_rust::prelude::*;
use adk_rust::session::{CreateRequest, SessionService};
use std::collections::HashMap;
use std::sync::Arc;

// ── Anthropic Token Counting & Model Discovery ──
// Demonstrates two Anthropic-specific APIs wrapped in an agentic flow:
// 1. Model discovery — list_models() and get_model() on AnthropicClient
// 2. Token counting — count_tokens() to estimate cost before sending
// Then runs the agent to show actual vs estimated token usage.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key =
        std::env::var("ANTHROPIC_API_KEY").expect("Set ANTHROPIC_API_KEY in your .env file");

    let model_name = "claude-sonnet-4-20250514";
    let client = AnthropicClient::new(
        AnthropicConfig::new(&api_key, model_name).with_max_tokens(512),
    )?;

    // ── Part 1: Model Discovery ──
    println!("=== Part 1: Model Discovery ===\n");

    let models = client.list_models().await?;
    println!("📋 Available Claude models ({} total):", models.len());
    for m in models.iter().take(8) {
        println!("   {} — {}", m.id, m.display_name);
    }
    if models.len() > 8 {
        println!("   ... and {} more", models.len() - 8);
    }

    println!("\n📌 Active model details:");
    match client.get_model(model_name).await {
        Ok(info) => {
            println!("   ID:      {}", info.id);
            println!("   Name:    {}", info.display_name);
            println!("   Created: {}", info.created_at);
        }
        Err(e) => println!("   Could not fetch: {e}"),
    }

    // ── Part 2: Token Counting (pre-flight) ──
    println!("\n=== Part 2: Token Counting (Pre-flight) ===\n");

    let system_prompt = "You are a concise technical writer. Explain concepts in 2-3 sentences.";
    let user_prompt = "Explain the difference between Box, Rc, and Arc in Rust. \
                       When should I use each one?";

    let request = LlmRequest {
        model: String::new(),
        contents: vec![
            Content::new("system").with_text(system_prompt),
            Content::new("user").with_text(user_prompt),
        ],
        config: None,
        tools: HashMap::new(),
    };

    let count = client.count_tokens(&request).await?;
    println!("📏 Pre-flight token count: {} input tokens", count.input_tokens);
    println!("   System: \"{}\"", &system_prompt[..system_prompt.len().min(60)]);
    println!("   User:   \"{}\"", &user_prompt[..user_prompt.len().min(60)]);

    // Estimate cost (Claude Sonnet: $3/M input, $15/M output)
    let est_input_cost = count.input_tokens as f64 * 3.0 / 1_000_000.0;
    println!("   Estimated input cost: ${:.6}", est_input_cost);

    // ── Part 3: Run the Agent ──
    println!("\n=== Part 3: Agent Response (Actual Usage) ===\n");

    let model = Arc::new(AnthropicClient::new(
        AnthropicConfig::new(&api_key, model_name).with_max_tokens(512),
    )?);

    let agent = Arc::new(
        LlmAgentBuilder::new("rust_explainer")
            .instruction(system_prompt)
            .model(model)
            .build()?,
    );

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

    let message = Content::new("user").with_text(user_prompt);
    let mut stream = runner.run(uid, sid, message).await?;

    while let Some(event) = stream.next().await {
        let event = event?;
        if let Some(content) = &event.llm_response.content {
            for part in &content.parts {
                if let Some(text) = part.text() {
                    print!("{}", text);
                }
            }
        }
        if let Some(usage) = &event.llm_response.usage_metadata {
            if usage.total_token_count > 0 {
                println!("\n\n📊 Actual usage — input: {}, output: {}, total: {}",
                    usage.prompt_token_count, usage.candidates_token_count, usage.total_token_count);
                let actual_cost = (usage.prompt_token_count as f64 * 3.0
                    + usage.candidates_token_count as f64 * 15.0)
                    / 1_000_000.0;
                println!("   Actual cost: ${:.6}", actual_cost);
                println!("   Pre-flight estimate was {} input tokens (actual: {})",
                    count.input_tokens, usage.prompt_token_count);
            }
        }
    }

    // ── Rate Limit Info ──
    println!("\n=== Rate Limit Status ===");
    let rate = client.latest_rate_limit_info().await;
    if let Some(remaining) = rate.requests_remaining {
        println!("   Requests remaining: {}", remaining);
    }
    if let Some(remaining) = rate.tokens_remaining {
        println!("   Tokens remaining:   {}", remaining);
    }
    if rate.requests_remaining.is_none() && rate.tokens_remaining.is_none() {
        println!("   (no rate-limit headers received)");
    }

    Ok(())
}
