// Realtime Tools — Function Calling in Voice Conversations
//
// Demonstrates tool use with OpenAI Realtime API:
// The agent has weather and calculator tools. When asked a question
// that requires tools, it calls them mid-conversation and incorporates
// the results into its response — all over a single WebSocket.
//
// Requires: OPENAI_API_KEY

use adk_realtime::config::{RealtimeConfig, ToolDefinition};
use adk_realtime::events::ServerEvent;
use adk_realtime::runner::{FnToolHandler, RealtimeRunner};
use serde_json::json;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");
    let api_key = std::env::var("OPENAI_API_KEY")?;
    let model_id = "gpt-4o-mini-realtime-preview-2024-12-17";

    println!("=== Realtime Tools — Function Calling in Voice ===\n");

    // ── Define tools ──
    let weather_tool = ToolDefinition {
        name: "get_weather".into(),
        description: Some("Get current weather for a city".into()),
        parameters: Some(json!({
            "type": "object",
            "properties": {
                "city": { "type": "string", "description": "City name" }
            },
            "required": ["city"]
        })),
    };

    let calc_tool = ToolDefinition {
        name: "calculate".into(),
        description: Some("Evaluate a math expression".into()),
        parameters: Some(json!({
            "type": "object",
            "properties": {
                "expression": { "type": "string", "description": "Math expression like '15 + 25'" }
            },
            "required": ["expression"]
        })),
    };

    let time_tool = ToolDefinition {
        name: "get_time".into(),
        description: Some("Get current time in a timezone".into()),
        parameters: Some(json!({
            "type": "object",
            "properties": {
                "timezone": { "type": "string", "description": "Timezone like 'PST', 'JST', 'UTC'" }
            },
            "required": ["timezone"]
        })),
    };

    // ── Build runner with tools ──
    let model = adk_realtime::openai::OpenAIRealtimeModel::new(&api_key, model_id);

    let runner = RealtimeRunner::builder()
        .model(Arc::new(model))
        .config(
            RealtimeConfig::default()
                .with_instruction(
                    "You are a helpful assistant with weather, calculator, and time tools. \
                     Use them when needed. Be concise — 2-3 sentences max.",
                )
                .with_voice("shimmer")
                .with_modalities(vec!["text".to_string()]),
        )
        .tool(
            weather_tool,
            FnToolHandler::new(|call| {
                let city = call.arguments["city"].as_str().unwrap_or("unknown");
                println!("  🌤️ get_weather(\"{}\")", city);
                let (temp, cond) = match city.to_lowercase().as_str() {
                    "tokyo" => (75, "Partly cloudy"),
                    "london" => (58, "Overcast"),
                    "paris" => (63, "Light rain"),
                    "new york" => (68, "Clear"),
                    _ => (70, "Fair"),
                };
                Ok(json!({"city": city, "temp_f": temp, "condition": cond}))
            }),
        )
        .tool(
            calc_tool,
            FnToolHandler::new(|call| {
                let expr = call.arguments["expression"].as_str().unwrap_or("0");
                println!("  🧮 calculate(\"{}\")", expr);
                // Simple eval for demo
                let result = match expr {
                    "75 - 58" => "17",
                    "75 - 63" => "12",
                    _ => "42",
                };
                Ok(json!({"expression": expr, "result": result}))
            }),
        )
        .tool(
            time_tool,
            FnToolHandler::new(|call| {
                let tz = call.arguments["timezone"].as_str().unwrap_or("UTC");
                println!("  🕐 get_time(\"{}\")", tz);
                let time = match tz.to_uppercase().as_str() {
                    "PST" => "4:30 PM",
                    "JST" => "8:30 AM (+1)",
                    "GMT" | "UTC" => "12:30 AM",
                    "CET" => "1:30 AM",
                    _ => "12:00 PM",
                };
                Ok(json!({"timezone": tz, "current_time": time}))
            }),
        )
        .build()?;

    println!("📡 Connecting to OpenAI Realtime API...");
    runner.connect().await?;
    println!("✅ Connected\n");
    println!("Tools: get_weather, calculate, get_time\n");

    // ── Turn 1: Single tool call ──
    let q1 = "What's the weather in Tokyo right now?";
    println!("── Turn 1: Single Tool ──\n");
    println!("👤 {}\n", q1);
    runner.send_text(q1).await?;
    runner.create_response().await?;
    let r1 = collect_response(&runner).await;
    println!("🤖 {}\n", r1);

    // ── Turn 2: Multi-tool (weather + time) ──
    let q2 = "What's the weather in London and what time is it there?";
    println!("── Turn 2: Multi-Tool ──\n");
    println!("👤 {}\n", q2);
    runner.send_text(q2).await?;
    runner.create_response().await?;
    let r2 = collect_response(&runner).await;
    println!("🤖 {}\n", r2);

    runner.close().await?;

    println!("=== Realtime Tool Features ===");
    println!("• ToolDefinition with JSON Schema parameters");
    println!("• FnToolHandler — closure-based tool execution");
    println!("• RealtimeRunner auto-dispatches tool calls and sends results back");
    println!("• Multiple tools can be called in a single turn");
    println!("• Works over WebSocket — no HTTP round-trips for tool calls");
    Ok(())
}

async fn collect_response(runner: &RealtimeRunner) -> String {
    let mut text = String::new();
    let mut count = 0;
    while let Some(event) = runner.next_event().await {
        count += 1;
        match event {
            Ok(ServerEvent::TextDelta { delta, .. }) => text.push_str(&delta),
            Ok(ServerEvent::TranscriptDelta { delta, .. }) => text.push_str(&delta),
            Ok(ServerEvent::ResponseDone { .. }) => break,
            Ok(ServerEvent::FunctionCallDone { .. }) => {}
            Ok(ServerEvent::Error { error, .. }) => {
                text = format!("Error: {}", error.message);
                break;
            }
            Err(e) => {
                text = format!("Stream error: {}", e);
                break;
            }
            _ => {}
        }
        if count > 300 { break; }
    }
    text
}
