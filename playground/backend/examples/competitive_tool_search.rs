use adk_anthropic::ToolSearchConfig;
use adk_core::{SessionId, UserId};
use adk_rust::futures::StreamExt;
use adk_rust::prelude::*;
use adk_rust::session::{CreateRequest, SessionService};
use adk_tool::tool;
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

// ── Tool Search Config — Regex-Based Tool Filtering ──
// Demonstrates `ToolSearchConfig` from adk-anthropic:
//
// When an agent has many tools, Claude can struggle to pick the right one.
// ToolSearchConfig lets you filter which tools are visible to the model
// using regex patterns. This improves accuracy and reduces token usage.
//
// Pattern: "^(search|fetch)_.*" → only tools starting with search_ or fetch_
// are sent to the model. Other tools exist but are hidden from the LLM.

// ── Safe tools (will match the filter) ──

#[derive(Deserialize, JsonSchema)]
struct SearchArgs {
    /// Search query
    query: String,
}

/// Search the knowledge base for relevant documents.
#[tool]
async fn search_docs(args: SearchArgs) -> adk_tool::Result<serde_json::Value> {
    println!("  🔍 search_docs: \"{}\"", args.query);
    Ok(serde_json::json!({
        "results": [
            {"title": "Getting Started with Rust", "relevance": 0.95},
            {"title": "Ownership and Borrowing", "relevance": 0.87},
            {"title": "Error Handling Patterns", "relevance": 0.72}
        ],
        "total": 3
    }))
}

#[derive(Deserialize, JsonSchema)]
struct FetchArgs {
    /// URL or document ID to fetch
    source: String,
}

/// Fetch content from a URL or document store.
#[tool]
async fn fetch_content(args: FetchArgs) -> adk_tool::Result<serde_json::Value> {
    println!("  📥 fetch_content: \"{}\"", args.source);
    Ok(serde_json::json!({
        "source": args.source,
        "content": "Rust's ownership system ensures memory safety without garbage collection...",
        "length": 847
    }))
}

// ── Dangerous tools (will NOT match the filter) ──

#[derive(Deserialize, JsonSchema)]
struct DeleteArgs {
    /// Resource ID to delete
    id: String,
}

/// Delete a resource permanently. DANGEROUS — requires admin access.
#[tool]
async fn delete_resource(args: DeleteArgs) -> adk_tool::Result<serde_json::Value> {
    println!("  ⚠️ delete_resource called: {} — THIS SHOULD NOT HAPPEN", args.id);
    Ok(serde_json::json!({"deleted": args.id}))
}

#[derive(Deserialize, JsonSchema)]
struct ExecArgs {
    /// Command to execute
    command: String,
}

/// Execute a system command. DANGEROUS — arbitrary code execution.
#[tool]
async fn execute_command(args: ExecArgs) -> adk_tool::Result<serde_json::Value> {
    println!("  ⚠️ execute_command called: {} — THIS SHOULD NOT HAPPEN", args.command);
    Ok(serde_json::json!({"output": "blocked"}))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    println!("=== Tool Search Config — Regex-Based Tool Filtering ===\n");

    // ── Part 1: Demonstrate ToolSearchConfig matching ──
    println!("── Part 1: ToolSearchConfig Pattern Matching ──\n");

    let config = ToolSearchConfig::new("^(search|fetch)_.*");
    let tools = ["search_docs", "fetch_content", "delete_resource", "execute_command"];

    for tool_name in &tools {
        let matches = config.matches(tool_name).unwrap_or(false);
        let icon = if matches { "✅" } else { "🚫" };
        println!("  {} {} → {}", icon, tool_name, if matches { "ALLOWED" } else { "FILTERED" });
    }

    // ── Part 2: Agent with filtered tools ──
    println!("\n── Part 2: Agent with Tool Filtering ──\n");

    let api_key = std::env::var("ANTHROPIC_API_KEY")
        .expect("Set ANTHROPIC_API_KEY in your .env file");

    // Configure Anthropic with tool search filtering
    let anthropic_config = AnthropicConfig::new(&api_key, "claude-sonnet-4-20250514")
        .with_max_tokens(1024)
        .with_tool_search(ToolSearchConfig::new("^(search|fetch)_.*"));

    println!("🔧 AnthropicConfig with tool_search: ^(search|fetch)_.*");
    println!("   Only search_* and fetch_* tools are visible to Claude\n");

    let model = Arc::new(AnthropicClient::new(anthropic_config)?);

    let agent = Arc::new(
        LlmAgentBuilder::new("filtered_assistant")
            .instruction(
                "You are a research assistant. Use the available tools to search for \
                 documents and fetch content. Answer the user's question based on \
                 what you find. Be concise.",
            )
            .model(model)
            .tool(Arc::new(SearchDocs))
            .tool(Arc::new(FetchContent))
            .tool(Arc::new(DeleteResource))     // registered but filtered out
            .tool(Arc::new(ExecuteCommand))     // registered but filtered out
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

    let message = Content::new("user")
        .with_text("Find documents about Rust ownership and fetch the most relevant one.");

    print!("🤖 ");
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
    }

    println!("\n\n=== Key Features ===");
    println!("• ToolSearchConfig::new(regex) — filter tools by name pattern");
    println!("• AnthropicConfig::with_tool_search() — attach filter to model config");
    println!("• Dangerous tools (delete, execute) are registered but invisible to the LLM");
    println!("• Reduces token usage — fewer tool schemas sent to the model");
    println!("• Improves accuracy — model picks from a focused tool set");
    Ok(())
}
