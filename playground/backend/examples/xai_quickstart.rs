use adk_core::{SessionId, UserId};
use adk_rust::futures::StreamExt;
use adk_rust::prelude::*;
use adk_rust::session::{CreateRequest, SessionService};
use adk_tool::tool;
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

// ── xAI Grok-3-mini with Tool Use ──
// Showcases xAI's Grok model via OpenAI-compatible API.
// Grok-3-mini-fast is optimized for speed with strong tool-calling ability.
// Uses `OpenAICompatibleConfig::xai()` — one line to connect to xAI.

#[derive(Deserialize, JsonSchema)]
struct DebugArgs {
    /// Error message or stack trace
    error: String,
    /// Programming language
    language: String,
}

/// Analyze an error and suggest fixes.
#[tool]
async fn debug_error(args: DebugArgs) -> adk_tool::Result<serde_json::Value> {
    Ok(serde_json::json!({
        "error": args.error,
        "language": args.language,
        "analysis": {
            "error_type": "NullPointerException",
            "root_cause": "Accessing `.unwrap()` on a None value at line 42",
            "related_issues": ["Missing null check", "No error propagation"],
            "suggested_fix": "Replace `.unwrap()` with `.ok_or_else(|| anyhow!(\"missing value\"))?`"
        }
    }))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key = std::env::var("XAI_API_KEY").expect("Set XAI_API_KEY in your .env file");

    // Grok-3-mini-fast: xAI's fast model with strong tool calling
    let model = Arc::new(OpenAICompatible::new(OpenAICompatibleConfig::xai(
        api_key,
        "grok-3-mini-fast",
    ))?);

    let agent = Arc::new(
        LlmAgentBuilder::new("debug_assistant")
            .instruction(
                "You are a debugging assistant powered by Grok. Analyze errors using the \
                 debug_error tool, then explain the root cause and provide a clear fix. \
                 Be direct and practical.",
            )
            .model(model)
            .tool(Arc::new(DebugError))
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

    println!("🔧 Grok-3-mini — Debugging Assistant\n");

    let message = Content::new("user").with_text(
        "I'm getting this Rust error: `thread 'main' panicked at 'called `Option::unwrap()` \
             on a `None` value', src/main.rs:42:37`. The code is trying to parse a config file. \
             Help me debug this.",
    );
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
