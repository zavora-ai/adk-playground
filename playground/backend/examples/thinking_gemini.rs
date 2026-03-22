use adk_rust::prelude::*;
use adk_rust::session::{SessionService, CreateRequest};
use adk_rust::futures::StreamExt;
use adk_core::{UserId, SessionId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

// ── Gemini Thinking & Thought Signatures ──
// Gemini 2.5+ models think natively — no special config needed.
// The model's internal reasoning appears as `Part::Thinking` blocks
// with an optional `signature` field.
//
// Key concepts:
//   - `Part::Thinking { thinking, signature }` — model's reasoning trace
//   - `Part::FunctionCall { thought_signature }` — links tool calls to
//     the reasoning that produced them. ADK preserves this across turns.
//   - `thinking_token_count` in usage metadata — tracks reasoning cost
//   - No config needed — Gemini 2.5 Flash thinks automatically on hard problems
//
// This example demonstrates:
//   1. Thinking traces on a multi-step math problem
//   2. Thought signatures on tool calls (tool thinking)
//   3. Multi-turn with preserved thought context

#[derive(JsonSchema, Serialize, Deserialize)]
struct CalculateArgs {
    /// Mathematical expression to evaluate
    expression: String,
}

#[derive(JsonSchema, Serialize, Deserialize)]
struct ConvertArgs {
    /// Numeric value to convert
    value: f64,
    /// Source unit (e.g. "km/h", "celsius", "kg")
    from_unit: String,
    /// Target unit (e.g. "mph", "fahrenheit", "lbs")
    to_unit: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key = std::env::var("GOOGLE_API_KEY")?;

    let calc_tool = FunctionTool::new(
        "calculate",
        "Evaluate a mathematical expression and return the numeric result",
        |_ctx, args| async move {
            let expr = args.get("expression").and_then(|v| v.as_str()).unwrap_or("0");
            let result: f64 = match expr {
                "120 / 2.25" | "120/2.25" => 53.33,
                "53.33 * 0.621371" | "53.33*0.621371" => 33.14,
                "53.33 * 1.60934" | "53.33*1.60934" => 85.84,
                "330 - 60" => 270.0,
                "270 / 150" => 1.8,
                "60 / 33.14" => 1.81,
                _ => expr.parse().unwrap_or(0.0),
            };
            Ok(serde_json::json!({ "expression": expr, "result": result }))
        },
    )
    .with_parameters_schema::<CalculateArgs>();

    let convert_tool = FunctionTool::new(
        "unit_convert",
        "Convert a value between units (distance, speed, temperature, weight)",
        |_ctx, args| async move {
            let value = args.get("value").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let from = args.get("from_unit").and_then(|v| v.as_str()).unwrap_or("?");
            let to = args.get("to_unit").and_then(|v| v.as_str()).unwrap_or("?");
            let result = match (from, to) {
                ("km/h", "mph") => value * 0.621371,
                ("mph", "km/h") => value * 1.60934,
                ("km", "miles") => value * 0.621371,
                ("miles", "km") => value * 1.60934,
                ("celsius", "fahrenheit") => value * 9.0 / 5.0 + 32.0,
                ("fahrenheit", "celsius") => (value - 32.0) * 5.0 / 9.0,
                ("kg", "lbs") => value * 2.20462,
                ("lbs", "kg") => value * 0.453592,
                _ => value,
            };
            Ok(serde_json::json!({
                "value": value, "from": from, "to": to,
                "result": format!("{result:.2}")
            }))
        },
    )
    .with_parameters_schema::<ConvertArgs>();

    // Gemini 2.5 Flash — thinking is built-in, no special config needed
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-2.5-flash")?);

    let agent = Arc::new(
        LlmAgentBuilder::new("math_thinker")
            .instruction(
                "You are a precise math assistant. Think through problems carefully. \
                 Use the calculate tool for arithmetic and unit_convert for conversions. \
                 Show your reasoning and verify results."
            )
            .model(model)
            .tool(Arc::new(calc_tool))
            .tool(Arc::new(convert_tool))
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
        session_service: sessions.clone(),
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

    println!("## 💎 Gemini Thinking — Thought Signatures & Tool Reasoning\n");

    // ── Turn 1: Multi-step problem triggers thinking + tool calls ──
    println!("### Turn 1: Multi-step calculation\n");
    let prompt1 = "A train travels 120 km in 2 hours and 15 minutes. \
             What is its average speed in both km/h and mph?";
    println!("<!--USER_PROMPT_START-->\n{}\n<!--USER_PROMPT_END-->", prompt1);
    let message = Content::new("user").with_text(prompt1);
    let mut stream = runner.run(UserId::new("user")?, SessionId::new("s1")?, message).await?;

    let mut thinking_count = 0;
    let mut tool_calls = 0;
    let mut thought_sigs = 0;

    while let Some(event) = stream.next().await {
        let event = event?;
        if let Some(content) = &event.llm_response.content {
            for part in &content.parts {
                match part {
                    Part::Thinking { thinking, .. } => {
                        thinking_count += 1;
                        println!("<!--THINKING_START-->\n{}\n<!--THINKING_END-->", thinking);
                    }
                    Part::FunctionCall { name, args, thought_signature, .. } => {
                        tool_calls += 1;
                        println!("\n🔧 `{}({})`\n", name, args);
                        if let Some(sig) = thought_signature {
                            thought_sigs += 1;
                            println!("🔗 thought\\_signature: `{}...` ({} chars)\n",
                                &sig[..sig.len().min(40)], sig.len());
                        }
                    }
                    Part::FunctionResponse { function_response, .. } => {
                        println!("📋 `{}`\n", function_response.response);
                    }
                    _ => {
                        if let Some(text) = part.text() { print!("{}", text); }
                    }
                }
            }
        }
        if event.llm_response.turn_complete {
            if let Some(usage) = &event.llm_response.usage_metadata {
                if let Some(thinking_tokens) = usage.thinking_token_count {
                    println!("\n\n📊 Thinking tokens used: **{}**\n", thinking_tokens);
                }
            }
        }
    }

    println!("\n---\n");
    println!("**Turn 1 Summary:** {} thinking blocks, {} tool calls, {} thought signatures\n",
        thinking_count, tool_calls, thought_sigs);

    // ── Turn 2: Follow-up relies on preserved history + thought signatures ──
    println!("### Turn 2: Follow-up (history includes thought signatures)\n");
    let prompt2 = "Now convert that speed to a pace (minutes per mile) for a runner comparison.";
    println!("<!--USER_PROMPT_START-->\n{}\n<!--USER_PROMPT_END-->", prompt2);
    let message = Content::new("user").with_text(prompt2);
    let mut stream = runner.run(UserId::new("user")?, SessionId::new("s1")?, message).await?;

    while let Some(event) = stream.next().await {
        let event = event?;
        if let Some(content) = &event.llm_response.content {
            for part in &content.parts {
                match part {
                    Part::Thinking { thinking, .. } => {
                        println!("<!--THINKING_START-->\n{}\n<!--THINKING_END-->", thinking);
                    }
                    Part::FunctionCall { name, args, .. } => {
                        println!("\n🔧 `{}({})`\n", name, args);
                    }
                    Part::FunctionResponse { function_response, .. } => {
                        println!("📋 `{}`\n", function_response.response);
                    }
                    _ => {
                        if let Some(text) = part.text() { print!("{}", text); }
                    }
                }
            }
        }
    }
    println!();
    Ok(())
}
