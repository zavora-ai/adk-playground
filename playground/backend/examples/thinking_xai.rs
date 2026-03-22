use adk_rust::prelude::*;
use adk_rust::session::{SessionService, CreateRequest};
use adk_rust::futures::StreamExt;
use adk_core::{UserId, SessionId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

// ── xAI Grok Thinking — OpenAI-Compatible Reasoning ──
// Grok-3-mini is xAI's reasoning model. It thinks step-by-step before
// answering, returning its chain-of-thought as `Part::Thinking` blocks
// via the OpenAI-compatible `reasoning_content` field.
//
// Key concepts:
//   - `grok-3-mini` — reasoning model (vs `grok-3-mini-fast` which skips thinking)
//   - `Part::Thinking` — contains the model's reasoning trace
//   - Works through `OpenAICompatible` — same ADK interface as any provider
//   - Reasoning tokens are billed separately and shown in usage metrics
//
// This example uses Grok's thinking to solve a Fermi estimation
// problem that requires careful reasoning with tool verification.

#[derive(JsonSchema, Serialize, Deserialize)]
struct EstimateArgs {
    /// Description of what is being estimated
    item: String,
    /// The estimated numeric value
    value: f64,
    /// Unit of measurement
    unit: String,
}

#[derive(JsonSchema, Serialize, Deserialize)]
struct CrossCheckArgs {
    /// The estimates to cross-check against each other
    estimates: Vec<String>,
    /// Whether the estimates are consistent
    consistent: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key = std::env::var("XAI_API_KEY")
        .expect("Set XAI_API_KEY in your .env file");

    // grok-3-mini: xAI's reasoning model with visible thinking
    let model = Arc::new(OpenAICompatible::new(
        OpenAICompatibleConfig::xai(api_key, "grok-3-mini")
    )?);

    let estimate_tool = FunctionTool::new(
        "record_estimate",
        "Record a Fermi estimation step with a numeric value and unit",
        |_ctx, args| async move {
            let item = args.get("item").and_then(|v| v.as_str()).unwrap_or("?");
            let value = args.get("value").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let unit = args.get("unit").and_then(|v| v.as_str()).unwrap_or("?");
            Ok(serde_json::json!({
                "recorded": true,
                "item": item,
                "value": value,
                "unit": unit,
            }))
        },
    )
    .with_parameters_schema::<EstimateArgs>();

    let crosscheck_tool = FunctionTool::new(
        "cross_check",
        "Cross-check multiple estimates for consistency",
        |_ctx, args| async move {
            let consistent = args.get("consistent").and_then(|v| v.as_bool()).unwrap_or(true);
            let estimates: Vec<String> = args.get("estimates")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();
            Ok(serde_json::json!({
                "estimates_checked": estimates.len(),
                "consistent": consistent,
                "status": if consistent { "pass" } else { "revise" },
            }))
        },
    )
    .with_parameters_schema::<CrossCheckArgs>();

    let agent = Arc::new(
        LlmAgentBuilder::new("fermi_estimator")
            .instruction(
                "You are a Fermi estimation expert. Break problems into smaller estimable \
                 quantities. Use record_estimate to log each sub-estimate, then use \
                 cross_check to verify consistency. Show your reasoning clearly."
            )
            .model(model)
            .tool(Arc::new(estimate_tool))
            .tool(Arc::new(crosscheck_tool))
            .build()?
    );

    let sessions = Arc::new(InMemorySessionService::new());
    sessions.create(CreateRequest {
        app_name: "playground".into(),
        user_id: "user".into(),
        session_id: Some("s1".into()),
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

    println!("## 🧠 xAI Grok-3-mini — OpenAI-Compatible Reasoning\n");
    println!("Grok reasons internally before responding (reasoning\\_content → Part::Thinking).\n");

    let message = Content::new("user")
        .with_text(
            "How many piano tuners are there in Chicago? \
             This is a classic Fermi estimation problem. Break it down step by step, \
             record each sub-estimate with the tool, then cross-check your estimates \
             for consistency before giving a final answer."
        );
    let mut stream = runner.run(UserId::new("user")?, SessionId::new("s1")?, message).await?;

    let mut thinking_blocks = 0;
    while let Some(event) = stream.next().await {
        let event = event?;
        if let Some(content) = &event.llm_response.content {
            for part in &content.parts {
                match part {
                    Part::Thinking { thinking, .. } => {
                        thinking_blocks += 1;
                        println!("<!--THINKING_START-->\n{}\n<!--THINKING_END-->", thinking);
                    }
                    _ => {
                        if let Some(text) = part.text() { print!("{}", text); }
                    }
                }
            }
        }
    }
    if thinking_blocks > 0 {
        println!("\n\n📊 Grok produced {} thinking block(s)", thinking_blocks);
    }
    println!();
    Ok(())
}
