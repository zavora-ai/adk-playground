use adk_core::{Part, RunConfig, SessionId, StreamingMode, UserId};
use adk_model::openai::{OpenAIResponsesClient, OpenAIResponsesConfig};
use adk_rust::futures::StreamExt;
use adk_rust::prelude::*;
use adk_rust::session::{CreateRequest, SessionService};
use adk_tool::{OpenAIApproximateLocation, OpenAIWebSearchTool};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

// ── OpenAI Built-in Tools — Multi-Search Research ──
//
// This example uses the first-class OpenAI native tool wrapper instead of
// manually stuffing JSON into GenerateContentConfig extensions.
//
// What it demonstrates:
//   1. Multiple hosted web searches to cross-reference and verify results
//   2. Local function tools coexisting with the hosted tool
//   3. Multi-turn conversation with research → structure → verify flow
//   4. Verification search to confirm or correct initial findings

const THIN: &str = "────────────────────────────────────────────────────────";
const THICK: &str = "════════════════════════════════════════════════════════";
const MODEL_NAME: &str = "gpt-5.4";

fn api_key() -> String {
    std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY must be set")
}

fn model_name() -> String {
    std::env::var("OPENAI_MODEL").unwrap_or_else(|_| MODEL_NAME.to_string())
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
    println!("  🔎 OpenAI Built-in Tools — Multi-Search Research");
    println!("  Multiple searches + cross-reference + verify");
    println!("{THICK}");

    let model = Arc::new(OpenAIResponsesClient::new(OpenAIResponsesConfig::new(
        api_key(),
        model_name(),
    ))?);

    let record_tool: Arc<dyn Tool> = Arc::new(
        FunctionTool::new(
            "record_tool_status",
            "Record which hosted tool was used and why.",
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

    let web_search_tool: Arc<dyn Tool> = Arc::new(
        OpenAIWebSearchTool::new()
            .with_search_context_size("medium")
            .with_user_location(
                OpenAIApproximateLocation::new()
                    .with_city("San Francisco")
                    .with_region("California")
                    .with_country("US")
                    .with_timezone("America/Los_Angeles"),
            ),
    );

    let agent = Arc::new(
        LlmAgentBuilder::new("research_agent")
            .instruction(
                "You are a thorough research assistant with OpenAI hosted web search.\n\
                 IMPORTANT: When researching a topic, perform MULTIPLE web searches to \
                 cross-reference and verify information. Do not rely on a single search.\n\
                 Strategy:\n\
                 1. First search for the topic directly\n\
                 2. If the first result is unclear, search again with different terms\n\
                 3. Search official sources (e.g. blog.rust-lang.org, releases pages)\n\
                 4. Only report findings once you have a confident, verified answer\n\n\
                 If asked, call record_tool_status after each search step.\n\
                 Use release_brief when the user asks for a structured brief.\n\
                 Always include specific version numbers, dates, and sources in your answers.",
            )
            .model(model)
            .tool(web_search_tool)
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
        run_config: Some(RunConfig {
            streaming_mode: StreamingMode::None,
            ..Default::default()
        }),
        compaction_config: None,
        context_cache_config: None,
        cache_capable: None,
        request_context: None,
        cancellation_token: None,
    })?;

    let uid = UserId::new("user")?;
    let sid = SessionId::new("s1")?;

    println!("\n{THICK}");
    println!("  TURN 1 — Multi-Search Research");
    println!("{THICK}");

    let q1 = "Find the latest stable Rust release version and its release date. \
              Perform multiple web searches to verify: first search for 'latest Rust release', \
              then search 'blog.rust-lang.org' for the official announcement. \
              After each search, call record_tool_status with tool_name 'web_search' \
              and a note about what that search found. \
              Only give your final answer once you're confident in the version number.";
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
                            .get("type")
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

    let q2 = "Now use release_brief to turn your previous answer into structured data. \
              Include the Rust version, release date if you know it, and one highlight.";
    println!("\n  👤 User: {q2}\n");
    print!("  🤖 Agent: ");

    let mut s2 = runner
        .run(uid.clone(), sid.clone(), Content::new("user").with_text(q2))
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

    // ── Turn 3: Verification search ──
    println!("\n{THICK}");
    println!("  TURN 3 — Verification Search");
    println!("{THICK}");

    let q3 = "Now verify your answer by searching for 'Rust changelog' or \
              'Rust release history' to confirm the version and date you reported. \
              If you find a discrepancy, correct the version. \
              Then call record_tool_status with tool_name 'web_search' and a note \
              about whether the verification confirmed or corrected your answer.";
    println!("\n  👤 User: {q3}\n");
    print!("  🤖 Agent: ");

    let mut s3 = runner
        .run(uid, sid, Content::new("user").with_text(q3))
        .await?;

    let mut t3_text = String::new();
    while let Some(event) = s3.next().await {
        let event = event?;
        if let Some(content) = &event.llm_response.content {
            for part in &content.parts {
                match part {
                    Part::ServerToolCall { server_tool_call } => {
                        let item_type = server_tool_call
                            .get("type")
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
                        t3_text.push_str(text);
                    }
                    _ => {}
                }
            }
        }
    }
    println!("\n\n  {THIN}");
    println!("  ✅ Turn 3 complete — {} chars", t3_text.len());

    println!("\n{THICK}");
    println!("  ✅ All turns completed successfully");
    println!("  • Multiple web searches used to cross-reference results");
    println!("  • Hosted tool output and local function tools coexisted");
    println!("  • Verification search confirmed or corrected initial findings");
    println!("  • Multi-turn conversation maintained context across 3 turns");
    println!("{THICK}");
    Ok(())
}
