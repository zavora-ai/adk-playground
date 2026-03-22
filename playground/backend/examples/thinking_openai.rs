use adk_rust::prelude::*;
use adk_rust::session::{SessionService, CreateRequest};
use adk_rust::futures::StreamExt;
use adk_core::{UserId, SessionId};
use adk_rust::model::openai::{
    OpenAIResponsesClient, OpenAIResponsesConfig,
    ReasoningEffort, ReasoningSummary,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

// ── OpenAI Responses API — Reasoning with Summaries ──
// The Responses API (`/v1/responses`) is OpenAI's latest endpoint,
// replacing Chat Completions for new development.
//
// `OpenAIResponsesClient` provides:
//   - True streaming (text + reasoning deltas)
//   - Native reasoning summaries via `ReasoningSummary`
//   - Tool calling with function tools
//   - Server-side conversation state via `previous_response_id`
//
// This example sends the SAME hard logic puzzle at each effort level
// so you can compare answer quality, reasoning depth, and latency.

#[derive(JsonSchema, Serialize, Deserialize)]
struct VerifyArgs {
    /// The logical statement to verify
    statement: String,
    /// Whether the statement is true or false
    verdict: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key = std::env::var("OPENAI_API_KEY")
        .expect("Set OPENAI_API_KEY in your .env file");

    let verify_tool = FunctionTool::new(
        "verify_logic",
        "Verify a logical deduction and record whether it is true or false",
        |_ctx, args| async move {
            let statement = args.get("statement").and_then(|v| v.as_str()).unwrap_or("?");
            let verdict = args.get("verdict").and_then(|v| v.as_bool()).unwrap_or(false);
            Ok(serde_json::json!({
                "recorded": true,
                "statement": statement,
                "verdict": verdict,
            }))
        },
    )
    .with_parameters_schema::<VerifyArgs>();
    let tool = Arc::new(verify_tool);

    // Hard logic puzzle that benefits from deeper reasoning
    let puzzle = "Three friends — Alice, Bob, and Carol — each have a different pet \
        (cat, dog, fish) and a different favorite color (red, blue, green).\n\
        Clues:\n\
        1. Alice does not have the cat.\n\
        2. The person with the dog likes blue.\n\
        3. Carol likes green.\n\
        4. Bob does not have the fish.\n\
        Determine who has which pet and which color. \
        Use the verify_logic tool to record each deduction, then give the final answer.";

    println!("🧠 OpenAI Responses API — Reasoning Effort Comparison\n");
    println!("Using OpenAIResponsesClient (POST /v1/responses)");
    println!("Model: o4-mini with ReasoningSummary::Detailed\n");
    println!("<!--USER_PROMPT_START-->\n{}\n<!--USER_PROMPT_END-->", puzzle);
    println!("{}\n", "─".repeat(60));

    for effort in [ReasoningEffort::Low, ReasoningEffort::Medium, ReasoningEffort::High] {
        let label = match effort {
            ReasoningEffort::Low => "Low (fast, minimal thinking)",
            ReasoningEffort::Medium => "Medium (balanced)",
            ReasoningEffort::High => "High (deep multi-step reasoning)",
            _ => "Unknown",
        };
        println!("── Effort: {} ──\n", label);

        let config = OpenAIResponsesConfig::new(&api_key, "o4-mini")
            .with_reasoning_effort(effort)
            .with_reasoning_summary(ReasoningSummary::Detailed);
        let model = Arc::new(OpenAIResponsesClient::new(config)?);

        let agent = Arc::new(
            LlmAgentBuilder::new("logic_solver")
                .instruction(
                    "You are a logic puzzle solver. Work through the clues step by step. \
                     Use verify_logic to record each deduction you make. \
                     After all deductions, state the final answer clearly."
                )
                .model(model)
                .tool(tool.clone())
                .build()?
        );

        let sessions = Arc::new(InMemorySessionService::new());
        let sid = format!("effort-{:?}", effort).to_lowercase();
        sessions.create(CreateRequest {
            app_name: "playground".into(),
            user_id: "user".into(),
            session_id: Some(sid.clone()),
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

        let start = std::time::Instant::now();
        let message = Content::new("user").with_text(puzzle);
        let mut stream = runner.run(UserId::new("user")?, SessionId::new(&sid)?, message).await?;

        while let Some(event) = stream.next().await {
            let event = event?;
            if let Some(content) = &event.llm_response.content {
                for part in &content.parts {
                    match part {
                        Part::Thinking { thinking, .. } => {
                            println!("<!--THINKING_START-->\n{}\n<!--THINKING_END-->", thinking);
                        }
                        _ => {
                            if let Some(text) = part.text() { print!("{}", text); }
                        }
                    }
                }
            }
        }
        println!("\n\n⏱  Completed in {:.1}s\n", start.elapsed().as_secs_f64());
        println!("{}\n", "─".repeat(60));
    }

    Ok(())
}
