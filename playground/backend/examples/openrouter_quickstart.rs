use adk_core::{GenerateContentConfig, SessionId, UserId};
use adk_rust::futures::StreamExt;
use adk_rust::model::openrouter::{
    OpenRouterApiMode, OpenRouterClient, OpenRouterConfig, OpenRouterProviderPreferences,
    OpenRouterRequestOptions,
};
use adk_rust::prelude::*;
use adk_rust::session::{CreateRequest, SessionService};
use adk_tool::tool;
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

// ── OpenRouter — Multi-Provider AI Gateway ──
// OpenRouter routes requests to 200+ models from OpenAI, Anthropic,
// Google, Meta, Mistral, and more — all through a single API key.
//
// Key concepts:
//   - `OpenRouterConfig::new(key, model)` — configure with any model ID
//   - `OpenRouterApiMode::ChatCompletions` — standard chat mode
//   - `OpenRouterRequestOptions` — provider routing, fallbacks, preferences
//   - Model IDs use `provider/model` format (e.g. "openai/gpt-4.1-mini")
//
// This example demonstrates:
//   1. Basic chat with function tool use
//   2. Provider routing with automatic fallback

#[derive(Deserialize, JsonSchema)]
struct LookupArgs {
    /// The technology or concept to look up
    topic: String,
}

/// Look up key facts about a technology.
#[tool]
async fn tech_lookup(args: LookupArgs) -> adk_tool::Result<serde_json::Value> {
    let info = match args.topic.to_lowercase() {
        t if t.contains("rust") => serde_json::json!({
            "topic": "Rust",
            "category": "Systems Programming Language",
            "created_by": "Graydon Hoare / Mozilla Research",
            "first_stable": "2015",
            "key_features": ["Memory safety without GC", "Zero-cost abstractions", "Fearless concurrency"],
            "use_cases": ["Systems programming", "WebAssembly", "CLI tools", "Embedded systems"]
        }),
        t if t.contains("openrouter") => serde_json::json!({
            "topic": "OpenRouter",
            "category": "AI Gateway / Model Router",
            "key_features": ["200+ models", "Automatic fallback", "Provider routing", "Usage-based pricing"],
            "supported_providers": ["OpenAI", "Anthropic", "Google", "Meta", "Mistral", "xAI"]
        }),
        _ => serde_json::json!({
            "topic": args.topic,
            "category": "Unknown",
            "note": "No detailed info available for this topic"
        }),
    };
    Ok(info)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key =
        std::env::var("OPENROUTER_API_KEY").expect("Set OPENROUTER_API_KEY in your .env file");

    let model_name = std::env::var("OPENROUTER_MODEL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "nvidia/nemotron-3-super-120b-a12b:free".into());

    // ── Part 1: Basic chat with tool use ──
    println!("## 🌐 OpenRouter — Multi-Provider AI Gateway\n");
    println!("Model: {model_name}\n");

    let config = OpenRouterConfig::new(&api_key, &model_name)
        .with_http_referer("https://github.com/zavora-ai/adk-rust")
        .with_title("ADK-Rust Playground");
    let model = Arc::new(OpenRouterClient::new(config)?);

    let agent = Arc::new(
        LlmAgentBuilder::new("tech_researcher")
            .instruction(
                "You are a concise tech researcher. Use the tech_lookup tool to find \
                 information, then summarize the key points clearly. Keep responses brief.",
            )
            .model(model)
            .tool(Arc::new(TechLookup))
            .build()?,
    );

    let sessions = Arc::new(InMemorySessionService::new());
    sessions
        .create(CreateRequest {
            app_name: "playground".into(),
            user_id: "user".into(),
            session_id: Some("s1".into()),
            state: HashMap::new(),
        })
        .await?;

    let runner = Runner::new(RunnerConfig {
        app_name: "playground".into(),
        agent: agent.clone(),
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

    let message = Content::new("user")
        .with_text("Look up Rust and OpenRouter, then compare them in a brief summary.");
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
    println!("\n");

    // ── Part 2: Provider routing with fallback ──
    println!("---\n## 🔀 Provider Routing with Fallback\n");

    let options = OpenRouterRequestOptions::default()
        .with_models(vec![model_name.clone(), "google/gemma-3-4b-it:free".into()])
        .with_route("fallback")
        .with_provider_preferences(OpenRouterProviderPreferences {
            allow_fallbacks: Some(true),
            ..Default::default()
        });

    let mut gen_config = GenerateContentConfig {
        max_output_tokens: Some(256),
        ..Default::default()
    };
    options.insert_into_config(&mut gen_config)?;

    let routing_config = OpenRouterConfig::new(&api_key, &model_name)
        .with_http_referer("https://github.com/zavora-ai/adk-rust")
        .with_title("ADK-Rust Playground")
        .with_default_api_mode(OpenRouterApiMode::ChatCompletions);
    let routing_model = Arc::new(OpenRouterClient::new(routing_config)?);

    let routing_agent = Arc::new(
        LlmAgentBuilder::new("routing_demo")
            .instruction("Answer in one concise sentence.")
            .model(routing_model)
            .generate_content_config(gen_config)
            .build()?,
    );

    sessions
        .create(CreateRequest {
            app_name: "playground".into(),
            user_id: "user".into(),
            session_id: Some("s2".into()),
            state: HashMap::new(),
        })
        .await?;

    let routing_runner = Runner::new(RunnerConfig {
        app_name: "playground".into(),
        agent: routing_agent,
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

    println!("Fallback chain: [{model_name}, google/gemma-3-4b-it:free]");
    println!("If the primary model is unavailable, OpenRouter automatically falls back.\n");

    let message = Content::new("user")
        .with_text("Why is automatic model fallback important for production AI systems?");
    let mut stream = routing_runner
        .run(UserId::new("user")?, SessionId::new("s2")?, message)
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
