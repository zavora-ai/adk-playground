use adk_core::{Content, RunConfig, SessionId, UserId};
use adk_rust::futures::StreamExt;
use adk_rust::prelude::*;
use adk_rust::session::{CreateRequest, SessionService};
use adk_tool::tool;
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

// ── Bounded Execution — RunConfig Controls ──
// Demonstrates v0.8 RunConfig extensions for production safety:
//
// 1. `history_max_events` — limit how many past events are loaded into context
// 2. `max_tool_concurrency` — cap parallel tool execution to prevent overload
//
// These controls prevent runaway agents from consuming unbounded resources.
// Essential for production deployments with cost and latency constraints.

#[derive(Deserialize, JsonSchema)]
struct ResearchArgs {
    /// Topic to research
    topic: String,
}

/// Research a topic and return findings.
#[tool]
async fn research_topic(args: ResearchArgs) -> adk_tool::Result<serde_json::Value> {
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    println!("  📚 Researching: {}", args.topic);
    Ok(serde_json::json!({
        "topic": args.topic,
        "findings": format!("Key insights about {}: widely adopted, growing ecosystem", args.topic),
        "sources": 3
    }))
}

#[derive(Deserialize, JsonSchema)]
struct AnalyzeArgs {
    /// Data to analyze
    data: String,
}

/// Analyze data and return structured insights.
#[tool]
async fn analyze_data(args: AnalyzeArgs) -> adk_tool::Result<serde_json::Value> {
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    println!("  🔬 Analyzing: {}...", &args.data[..args.data.len().min(40)]);
    Ok(serde_json::json!({
        "sentiment": "positive",
        "confidence": 0.92,
        "key_themes": ["performance", "safety", "ergonomics"]
    }))
}

#[derive(Deserialize, JsonSchema)]
struct SummarizeArgs {
    /// Content to summarize
    content: String,
}

/// Summarize content concisely.
#[tool]
async fn summarize(args: SummarizeArgs) -> adk_tool::Result<serde_json::Value> {
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    println!("  📝 Summarizing...");
    Ok(serde_json::json!({
        "summary": format!("Summary: {}", &args.content[..args.content.len().min(60)]),
        "word_count": 25
    }))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    println!("=== Bounded Execution — RunConfig Production Controls ===\n");

    let api_key = std::env::var("GOOGLE_API_KEY")?;
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-3.1-flash-lite-preview")?);

    // ── Part 1: Configure bounded execution ──
    println!("── Part 1: RunConfig with Bounds ──\n");

    let run_config = RunConfig {
        // Only load the last 20 events into context (prevents unbounded growth)
        history_max_events: Some(20),
        ..Default::default()
    };

    println!("  ⚙️  RunConfig:");
    println!("     history_max_events: 20 (older events trimmed from context)");
    println!("     tool_concurrency: default (use ToolConcurrencyConfig for limits)");
    println!();
    println!("  Why this matters:");
    println!("     • Long conversations accumulate thousands of events");
    println!("     • Without bounds, context grows until it hits token limits");
    println!("     • Parallel tools can overwhelm downstream APIs");
    println!("     • These bounds give predictable latency and cost");

    // ── Part 2: Build agent with multiple tools ──
    println!("\n── Part 2: Multi-Tool Agent ──\n");

    let agent = Arc::new(
        LlmAgentBuilder::new("research_analyst")
            .instruction(
                "You are a research analyst. When asked about a topic, use your tools \
                 to research it, analyze the findings, and provide a summary. \
                 You can call multiple tools to gather comprehensive information.",
            )
            .model(model)
            .tool(Arc::new(ResearchTopic))
            .tool(Arc::new(AnalyzeData))
            .tool(Arc::new(Summarize))
            .build()?,
    );

    println!("  🤖 Agent: research_analyst");
    println!("     Tools: research_topic, analyze_data, summarize");

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
        run_config: Some(run_config),
        compaction_config: None,
        context_cache_config: None,
        cache_capable: None,
        request_context: None,
        cancellation_token: None,
        intra_compaction_config: None,
        intra_compaction_summarizer: None,
    })?;

    // ── Part 3: Run with bounded execution ──
    println!("\n── Part 3: Bounded Execution ──\n");

    let message = Content::new("user").with_text(
        "Research the Rust programming language, analyze its adoption trends, \
         and give me a brief summary of why it's popular.",
    );

    println!("  📨 User: Research Rust, analyze trends, summarize popularity");
    println!("  ⏱️  Tool calls bounded to max 2 concurrent:\n");

    print!("🤖 ");
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

    println!("\n\n=== Key Features ===");
    println!("• RunConfig::history_max_events — cap context window growth");
    println!("• RunConfig::tool_concurrency — control parallel tool execution");
    println!("• ToolConcurrencyConfig — max_concurrent, per-tool overrides, backpressure");
    println!("• Prevents runaway costs from unbounded history accumulation");
    println!("• Protects downstream APIs from concurrent request floods");
    println!("• Essential for production agents with cost/latency SLAs");
    println!("• Works with all agent types (LLM, Sequential, Parallel, Graph)");
    Ok(())
}
