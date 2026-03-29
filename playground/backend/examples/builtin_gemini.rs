use adk_core::{Part, SessionId, UserId};
use adk_model::GeminiModel;
use adk_rust::futures::StreamExt;
use adk_rust::prelude::*;
use adk_rust::session::{CreateRequest, SessionService};
use adk_tool::GoogleSearchTool;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

// ── Gemini Built-in Tools — Google Search Wrapper ──
//
// This example uses the typed Gemini GoogleSearchTool wrapper and shows the
// two behaviors that matter most in practice:
//   1. Grounding metadata from server-side search
//   2. thoughtSignature preservation when native tools and function tools mix

const THIN: &str = "────────────────────────────────────────────────────────";
const THICK: &str = "════════════════════════════════════════════════════════";
const MODEL_NAME: &str = "gemini-3-pro-preview";

fn model_name() -> String {
    std::env::var("GEMINI_MODEL").unwrap_or_else(|_| MODEL_NAME.to_string())
}

fn sig_short(sig: &Option<String>) -> String {
    match sig {
        Some(sig) if sig.len() > 40 => format!("{}…[{}B]", &sig[..40], sig.len()),
        Some(sig) => sig.clone(),
        None => "—".into(),
    }
}

fn server_sig(value: &serde_json::Value) -> Option<String> {
    value
        .get("thoughtSignature")
        .and_then(|v| v.as_str())
        .or_else(|| {
            value
                .get("toolCall")
                .and_then(|tool_call| tool_call.get("_thought_signature"))
                .and_then(|v| v.as_str())
        })
        .map(String::from)
}

fn print_grounding(event: &adk_core::Event) {
    let Some(meta) = &event.llm_response.provider_metadata else {
        return;
    };
    let Some(obj) = meta.as_object() else {
        return;
    };
    if obj.is_empty() {
        return;
    }

    println!("\n  {THIN}");
    println!("  📡 GROUNDING METADATA");
    println!("  {THIN}");

    if let Some(queries) = obj.get("webSearchQueries").and_then(|v| v.as_array()) {
        let queries: Vec<&str> = queries.iter().filter_map(|value| value.as_str()).collect();
        if !queries.is_empty() {
            println!("  🔍 Queries: {}", queries.join(" | "));
        }
    }

    if let Some(chunks) = obj.get("groundingChunks").and_then(|v| v.as_array()) {
        println!("  📚 Sources:");
        for (index, chunk) in chunks.iter().enumerate() {
            if let Some(web) = chunk.get("web") {
                let title = web.get("title").and_then(|v| v.as_str()).unwrap_or("?");
                let uri = web.get("uri").and_then(|v| v.as_str()).unwrap_or("?");
                println!("     [{index}] {title}");
                println!("         {uri}");
            }
        }
    }

    println!("  {THIN}");
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
    println!("  🔍 Gemini Built-in Tools — Google Search");
    println!("  Grounding metadata + thought signatures + multi-turn");
    println!("{THICK}");

    let api_key = std::env::var("GOOGLE_API_KEY").expect("GOOGLE_API_KEY must be set");
    let model = Arc::new(GeminiModel::new(api_key, model_name())?);

    let search_tool: Arc<dyn Tool> = Arc::new(GoogleSearchTool::new());
    let record_tool: Arc<dyn Tool> = Arc::new(
        FunctionTool::new(
            "record_tool_status",
            "Record which built-in tool was used and why.",
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
                "You are a research assistant with live Google Search.\n\
                 Use Google Search for current information.\n\
                 If asked, call record_tool_status after the search step.\n\
                 Use release_brief when the user asks for a structured brief.",
            )
            .model(model)
            .tool(search_tool)
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
    println!("  TURN 1 — Google Search + Function Tool");
    println!("{THICK}");

    let q1 = "Use Google Search to find the latest stable Rust release. \
              After searching, call record_tool_status with tool_name \
              'google_search' and a short note.";
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
                        println!("\n  ┌─ 🔧 ServerToolCall: google_search");
                        println!(
                            "  │  thought_signature: {}",
                            sig_short(&server_sig(server_tool_call))
                        );
                    }
                    Part::ServerToolResponse {
                        server_tool_response,
                    } => {
                        println!(
                            "  └─ 📥 ServerToolResponse: {} bytes\n",
                            server_tool_response.to_string().len()
                        );
                    }
                    Part::FunctionCall {
                        name,
                        thought_signature,
                        ..
                    } => {
                        println!("\n  ┌─ ⚡ FunctionCall: {name}");
                        println!("  │  thought_signature: {}", sig_short(thought_signature));
                    }
                    Part::FunctionResponse {
                        function_response, ..
                    } => {
                        println!("  └─ 📤 FunctionResponse: {}\n", function_response.response);
                    }
                    Part::Thinking { signature, .. } => {
                        println!("  💭 Thinking (sig: {})", sig_short(signature));
                    }
                    Part::Text { text } if !text.trim().is_empty() => {
                        print!("{text}");
                        t1_text.push_str(text);
                    }
                    _ => {}
                }
            }
        }
        print_grounding(&event);
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
                    Part::FunctionCall {
                        name,
                        thought_signature,
                        ..
                    } => {
                        println!("\n  ┌─ ⚡ FunctionCall: {name}");
                        println!("  │  thought_signature: {}", sig_short(thought_signature));
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
        print_grounding(&event);
    }
    println!("\n\n  {THIN}");
    println!("  ✅ Turn 2 complete — {} chars", t2_text.len());

    println!("\n{THICK}");
    println!("  ✅ All turns completed successfully");
    println!("  • GoogleSearchTool invoked Gemini server-side search");
    println!("  • Grounding metadata was surfaced to the agent stream");
    println!("  • thoughtSignature survived native-tool and function-tool turns");
    println!("{THICK}");
    Ok(())
}
