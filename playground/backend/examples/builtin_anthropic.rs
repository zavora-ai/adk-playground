use adk_core::{Part, SessionId, UserId};
use adk_model::anthropic::{AnthropicClient, AnthropicConfig};
use adk_rust::futures::StreamExt;
use adk_rust::prelude::*;
use adk_rust::session::{CreateRequest, SessionService};
use adk_tool::WebSearchTool;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

// ── Anthropic Built-in Tools — Native Web Search Wrapper ──
//
// This example uses the typed Anthropic web-search wrapper instead of a raw
// built_in_tools extension blob.
//
// What it demonstrates:
//   1. Claude server-side web search surfaced as ServerToolCall/Response parts
//   2. Local function tools coexisting with the native tool
//   3. Multi-turn continuity after a web-search turn

const THIN: &str = "────────────────────────────────────────────────────────";
const THICK: &str = "════════════════════════════════════════════════════════";
const MODEL_NAME: &str = "claude-sonnet-4-20250514";

fn api_key() -> String {
    std::env::var("ANTHROPIC_API_KEY").expect("ANTHROPIC_API_KEY must be set")
}

fn model_name() -> String {
    std::env::var("ANTHROPIC_MODEL").unwrap_or_else(|_| MODEL_NAME.to_string())
}

#[derive(schemars::JsonSchema, serde::Serialize)]
struct ToolStatusArgs {
    tool_name: String,
    note: String,
}

#[derive(schemars::JsonSchema, serde::Serialize)]
struct ReleaseBriefArgs {
    version: String,
    release_date: Option<String>,
    highlight: String,
}

async fn record_tool_status(
    _ctx: Arc<dyn ToolContext>,
    args: serde_json::Value,
) -> Result<serde_json::Value> {
    Ok(json!({
        "acknowledged": true,
        "tool_name": args["tool_name"].as_str().unwrap_or("unknown"),
        "note": args["note"].as_str().unwrap_or(""),
    }))
}

async fn release_brief(
    _ctx: Arc<dyn ToolContext>,
    args: serde_json::Value,
) -> Result<serde_json::Value> {
    Ok(json!({
        "version": args["version"].as_str().unwrap_or("unknown"),
        "release_date": args["release_date"].as_str(),
        "highlight": args["highlight"].as_str().unwrap_or(""),
        "status": "brief_created",
    }))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    println!("{THICK}");
    println!("  🔎 Anthropic Built-in Tools — Web Search");
    println!("  Server-side search + function tools + multi-turn");
    println!("{THICK}");

    let model = Arc::new(AnthropicClient::new(AnthropicConfig::new(
        api_key(),
        model_name(),
    ))?);

    let record_tool: Arc<dyn Tool> = Arc::new(
        FunctionTool::new(
            "record_tool_status",
            "Record which native tool was used and why.",
            record_tool_status,
        )
        .with_parameters_schema::<ToolStatusArgs>(),
    );

    let release_brief_tool: Arc<dyn Tool> = Arc::new(
        FunctionTool::new(
            "release_brief",
            "Turn release details into a structured brief.",
            release_brief,
        )
        .with_parameters_schema::<ReleaseBriefArgs>(),
    );

    let agent = Arc::new(
        LlmAgentBuilder::new("research_agent")
            .instruction(
                "You are a research assistant with Anthropic web search.\n\
                 Use native web search for current information.\n\
                 If asked, call record_tool_status after the search step.\n\
                 Use release_brief when the user asks for a structured brief.",
            )
            .model(model)
            .tool(Arc::new(WebSearchTool::new().with_max_uses(2)))
            .tool(record_tool)
            .tool(release_brief_tool)
            .build()?,
    );

    let sessions: Arc<dyn SessionService> = Arc::new(InMemorySessionService::new());
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

    let uid = UserId::new("user")?;
    let sid = SessionId::new("s1")?;

    println!("\n{THICK}");
    println!("  TURN 1 — Server-side Web Search");
    println!("{THICK}");

    let q1 = "Use Anthropic web search to find the latest stable Rust release. \
              After searching, call record_tool_status with tool_name \
              'web_search' and a short note.";
    println!("\n  👤 User: {q1}\n");
    print!("  🤖 Agent: ");

    let mut s1 = runner
        .run(uid.clone(), sid.clone(), Content::new("user").with_text(q1))
        .await?;

    let mut t1_text = String::new();
    while let Some(event) = s1.next().await {
        let event = event?;
        if let Some(content) = &event.llm_response.content {
            for part in &content.parts {
                match part {
                    Part::ServerToolCall { server_tool_call } => {
                        let item_type = server_tool_call
                            .get("name")
                            .or_else(|| server_tool_call.get("type"))
                            .and_then(|value| value.as_str())
                            .unwrap_or("unknown");
                        println!("\n  ┌─ 🔧 ServerToolCall: {item_type}");
                    }
                    Part::ServerToolResponse {
                        server_tool_response,
                    } => {
                        println!(
                            "  └─ 📥 ServerToolResponse: {} bytes\n",
                            server_tool_response.to_string().len()
                        );
                    }
                    Part::FunctionCall { name, .. } => {
                        println!("\n  ┌─ ⚡ FunctionCall: {name}");
                    }
                    Part::FunctionResponse {
                        function_response, ..
                    } => {
                        println!("  └─ 📤 FunctionResponse: {}\n", function_response.response);
                    }
                    Part::Text { text } if !text.trim().is_empty() => {
                        print!("{text}");
                        t1_text.push_str(text);
                    }
                    _ => {}
                }
            }
        }
    }
    println!("\n\n  {THIN}");
    println!("  ✅ Turn 1 complete — {} chars", t1_text.len());

    println!("\n{THICK}");
    println!("  TURN 2 — Structured Local Brief");
    println!("{THICK}");

    let q2 = "Now use release_brief to convert that answer into structured data. \
              Include the Rust version, release date if known, and one highlight.";
    println!("\n  👤 User: {q2}\n");
    print!("  🤖 Agent: ");

    let mut s2 = runner
        .run(uid, sid, Content::new("user").with_text(q2))
        .await?;

    let mut t2_text = String::new();
    while let Some(event) = s2.next().await {
        let event = event?;
        if let Some(content) = &event.llm_response.content {
            for part in &content.parts {
                match part {
                    Part::FunctionCall { name, .. } => {
                        println!("\n  ┌─ ⚡ FunctionCall: {name}");
                    }
                    Part::FunctionResponse {
                        function_response, ..
                    } => {
                        println!("  └─ 📤 FunctionResponse: {}\n", function_response.response);
                    }
                    Part::Text { text } if !text.trim().is_empty() => {
                        print!("{text}");
                        t2_text.push_str(text);
                    }
                    _ => {}
                }
            }
        }
    }
    println!("\n\n  {THIN}");
    println!("  ✅ Turn 2 complete — {} chars", t2_text.len());

    println!("\n{THICK}");
    println!("  ✅ All turns completed successfully");
    println!("  • WebSearchTool invoked Anthropic server-side search");
    println!("  • Native search and local function tools coexisted");
    println!("  • Multi-turn conversation continued after the search step");
    println!("{THICK}");
    Ok(())
}
