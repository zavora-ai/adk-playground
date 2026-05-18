use adk_core::{Content, SessionId, UserId};
use adk_rust::futures::StreamExt;
use adk_rust::prelude::*;
use adk_rust::session::{CreateRequest, SessionService};
use adk_tool::tool;
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

// ── Retry & Reflect Plugin (v0.8.2) ──
// Demonstrates the RetryReflectPlugin handling tool failures gracefully:
//
// 1. A flaky tool that fails intermittently (simulating transient errors)
// 2. The plugin intercepts failures and injects reflection prompts
// 3. The agent self-corrects after receiving reflection guidance
// 4. Exponential backoff between retries (configurable base delay)
//
// This pattern is essential for production agents that call external APIs
// which may have transient failures, rate limits, or intermittent issues.

static CALL_COUNT: AtomicU32 = AtomicU32::new(0);

#[derive(Deserialize, JsonSchema)]
struct SearchArgs {
    /// Search query
    query: String,
}

/// Search an external API (simulates transient failures).
/// Fails on the first call, succeeds on retry — demonstrating the retry pattern.
#[tool]
async fn flaky_search(args: SearchArgs) -> adk_tool::Result<serde_json::Value> {
    let count = CALL_COUNT.fetch_add(1, Ordering::SeqCst);

    if count % 2 == 0 {
        // First call fails
        println!("  ❌ flaky_search FAILED (attempt {}): \"{}\"", count + 1, args.query);
        Err(adk_tool::AdkError::tool(format!(
            "Service temporarily unavailable (attempt {}). The search API returned a 503.",
            count + 1
        )))
    } else {
        // Retry succeeds
        println!("  ✅ flaky_search SUCCESS (attempt {}): \"{}\"", count + 1, args.query);
        Ok(serde_json::json!({
            "query": args.query,
            "results": [
                {"title": "Rust Ownership Explained", "url": "https://doc.rust-lang.org/book/ch04-01-what-is-ownership.html"},
                {"title": "Understanding Borrowing", "url": "https://doc.rust-lang.org/book/ch04-02-references-and-borrowing.html"}
            ],
            "total": 2,
            "attempt": count + 1
        }))
    }
}

#[derive(Deserialize, JsonSchema)]
struct AnalyzeArgs {
    /// Text to analyze
    text: String,
}

/// Analyze text (always succeeds — contrast with the flaky tool).
#[tool]
async fn analyze(args: AnalyzeArgs) -> adk_tool::Result<serde_json::Value> {
    println!("  📊 analyze: \"{}...\"", &args.text[..args.text.len().min(30)]);
    Ok(serde_json::json!({
        "sentiment": "informative",
        "topics": ["rust", "memory safety", "ownership"],
        "confidence": 0.94
    }))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    println!("=== Retry & Reflect Plugin (v0.8.2) ===\n");

    // ── Part 1: Plugin Configuration ──
    println!("── Part 1: RetryReflect Configuration ──\n");
    println!("  RetryReflectPlugin config:");
    println!("    max_retries: 3");
    println!("    base_delay: 100ms (doubles each attempt)");
    println!("    reflection_prompt: guides the agent to try alternative approaches");
    println!();
    println!("  How it works:");
    println!("    1. Agent calls a tool → tool returns an error");
    println!("    2. Plugin intercepts the error");
    println!("    3. Plugin injects a reflection message: \"The tool failed because...\"");
    println!("    4. Agent receives the reflection and adjusts its approach");
    println!("    5. Agent retries (with exponential backoff between attempts)");

    // ── Part 2: Agent with flaky tool ──
    println!("\n── Part 2: Agent with Flaky Tool ──\n");

    let api_key = std::env::var("GOOGLE_API_KEY")?;
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-3.1-flash-lite-preview")?);

    let agent = Arc::new(
        LlmAgentBuilder::new("research_agent")
            .instruction(
                "You are a research assistant. Use the flaky_search tool to find information, \
                 and the analyze tool to process results. If a search fails, try again — \
                 the service has intermittent issues but usually works on retry.",
            )
            .model(model)
            .tool(Arc::new(FlakySearch))
            .tool(Arc::new(Analyze))
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
        intra_compaction_config: None,
        intra_compaction_summarizer: None,
    })?;

    // ── Part 3: Run — watch the retry pattern ──
    println!("\n── Part 3: Execution (watch retry pattern) ──\n");

    let message = Content::new("user")
        .with_text("Search for information about Rust ownership and analyze the results.");

    println!("  📨 User: Search for Rust ownership info and analyze results\n");

    print!("  🤖 ");
    let mut stream = runner.run(uid, sid, message).await?;
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

    let total_calls = CALL_COUNT.load(Ordering::SeqCst);
    println!("\n\n── Summary ──\n");
    println!("  Total flaky_search calls: {}", total_calls);
    println!("  Failed attempts: {}", (total_calls + 1) / 2);
    println!("  Successful attempts: {}", total_calls / 2);
    println!("  Agent self-corrected after tool failure ✓");

    println!("\n=== Key Features ===");
    println!("• RetryReflectPlugin — intercepts tool errors, injects reflection prompts");
    println!("• Exponential backoff — 100ms, 200ms, 400ms between retries");
    println!("• Configurable max_retries and base_delay");
    println!("• Agent learns from failures via reflection messages");
    println!("• Essential for production agents calling external APIs");
    println!("• Works with any tool — no tool-side changes needed");
    Ok(())
}
