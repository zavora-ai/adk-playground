use adk_rust::prelude::*;
use adk_rust::session::{SessionService, CreateRequest};
use adk_rust::futures::StreamExt;
use adk_core::{UserId, SessionId};
use adk_rust::model::openai::{
    OpenAIResponsesClient, OpenAIResponsesConfig, ReasoningEffort,
};
use adk_tool::tool;
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

// ── OpenAI Responses API — o4-mini with Tool Use ──
// Uses `OpenAIResponsesClient` (POST /v1/responses), OpenAI's latest endpoint.
// o4-mini applies chain-of-thought reasoning internally.
// `ReasoningEffort::Low` keeps it fast and cheap.

#[derive(Deserialize, JsonSchema)]
struct FactCheckArgs {
    /// The claim to verify
    claim: String,
}

/// Fact-check a claim and return a verdict with reasoning.
#[tool]
async fn fact_check(args: FactCheckArgs) -> adk_tool::Result<serde_json::Value> {
    let verdict = if args.claim.to_lowercase().contains("rust") {
        serde_json::json!({
            "claim": args.claim,
            "verdict": "TRUE",
            "evidence": "Rust was first released in 2015 by Mozilla Research. It has won Stack Overflow's 'most loved language' survey multiple years running.",
            "confidence": 0.95
        })
    } else {
        serde_json::json!({
            "claim": args.claim,
            "verdict": "UNVERIFIED",
            "evidence": "No matching records found in knowledge base.",
            "confidence": 0.3
        })
    };
    Ok(verdict)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key = std::env::var("OPENAI_API_KEY")
        .expect("Set OPENAI_API_KEY in your .env file");

    // o4-mini via Responses API — fast reasoning with configurable effort
    let config = OpenAIResponsesConfig::new(api_key, "o4-mini")
        .with_reasoning_effort(ReasoningEffort::Low);
    let model = Arc::new(OpenAIResponsesClient::new(config)?);

    let agent = Arc::new(
        LlmAgentBuilder::new("fact_checker")
            .instruction(
                "You are a fact-checking assistant. Use the fact_check tool to verify claims. \
                 Summarize the verdict clearly with the confidence level."
            )
            .model(model)
            .tool(Arc::new(FactCheck))
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

    println!("🧠 OpenAI o4-mini — Responses API with tool use\n");

    let message = Content::new("user")
        .with_text("Please fact-check: Rust is a systems programming language created by Mozilla.");
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
