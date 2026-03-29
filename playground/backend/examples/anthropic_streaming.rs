use adk_core::{SessionId, UserId};
use adk_rust::futures::StreamExt;
use adk_rust::prelude::*;
use adk_rust::session::{CreateRequest, SessionService};
use adk_tool::tool;
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

// ── Anthropic Streaming Agent — Real-Time Tool Use ──
// Demonstrates streaming with Claude: the agent streams its response token by
// token while also making tool calls mid-stream. Shows how streaming works
// with the Runner pattern — each SSE event arrives as it's generated.

#[derive(Deserialize, JsonSchema)]
struct LookupArgs {
    /// The programming concept to look up
    concept: String,
}

/// Look up a programming concept and return a brief definition.
#[tool]
async fn lookup_concept(args: LookupArgs) -> adk_tool::Result<serde_json::Value> {
    println!("  🔍 Looking up: {}", args.concept);
    let definition = match args.concept.to_lowercase().as_str() {
        s if s.contains("ownership") => "Rust's ownership system ensures memory safety without a garbage collector. Each value has exactly one owner, and the value is dropped when the owner goes out of scope.",
        s if s.contains("borrow") => "Borrowing lets you reference data without taking ownership. Immutable borrows (&T) allow multiple readers; mutable borrows (&mut T) allow exactly one writer.",
        s if s.contains("lifetime") => "Lifetimes are Rust's way of ensuring references don't outlive the data they point to. The compiler uses lifetime annotations to verify reference validity at compile time.",
        s if s.contains("trait") => "Traits define shared behavior. They're similar to interfaces in other languages but support default implementations, associated types, and can be used as bounds on generics.",
        s if s.contains("async") => "Async/await in Rust enables non-blocking I/O. Async functions return Futures that are lazily evaluated — they don't execute until polled by a runtime like Tokio.",
        _ => "A fundamental programming concept in Rust's type system.",
    };
    Ok(serde_json::json!({
        "concept": args.concept,
        "definition": definition
    }))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key =
        std::env::var("ANTHROPIC_API_KEY").expect("Set ANTHROPIC_API_KEY in your .env file");

    let model = Arc::new(AnthropicClient::new(
        AnthropicConfig::new(&api_key, "claude-sonnet-4-20250514")
            .with_max_tokens(1024),
    )?);

    let agent = Arc::new(
        LlmAgentBuilder::new("rust_tutor")
            .instruction(
                "You are a Rust programming tutor. Use the lookup_concept tool to \
                 fetch definitions for concepts you mention, then weave them into \
                 a clear explanation. Look up at least 2 concepts per response.",
            )
            .model(model)
            .tool(Arc::new(LookupConcept))
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

    println!("=== Anthropic Streaming — Real-Time Tool Use ===\n");
    println!("Streaming tokens as they arrive + tool calls mid-stream\n");

    let message = Content::new("user").with_text(
        "Explain how Rust's ownership and borrowing work together. \
         Look up each concept and give a beginner-friendly explanation.",
    );

    let start = std::time::Instant::now();
    let mut stream = runner.run(uid, sid, message).await?;
    let mut token_count = 0;
    let mut first_token_ms = None;

    while let Some(event) = stream.next().await {
        let event = event?;
        if let Some(content) = &event.llm_response.content {
            for part in &content.parts {
                if let Some(text) = part.text() {
                    if first_token_ms.is_none() {
                        first_token_ms = Some(start.elapsed().as_millis());
                    }
                    token_count += 1;
                    print!("{}", text);
                }
            }
        }
        if let Some(usage) = &event.llm_response.usage_metadata {
            if usage.total_token_count > 0 {
                println!("\n\n📊 Tokens — input: {}, output: {}",
                    usage.prompt_token_count, usage.candidates_token_count);
            }
        }
    }

    let total_ms = start.elapsed().as_millis();
    println!("\n\n⏱️  Timing:");
    if let Some(ttft) = first_token_ms {
        println!("   Time to first token: {}ms", ttft);
    }
    println!("   Total response time: {}ms", total_ms);
    println!("   Stream chunks: {}", token_count);

    Ok(())
}
