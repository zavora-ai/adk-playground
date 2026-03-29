// Realtime Session Update — Mid-Session Persona Switch
//
// Demonstrates OpenAI Realtime API's dynamic session update:
// Phase 1: General assistant answers a question
// Phase 2: Mid-session switch to travel agent persona with new tools
// The WebSocket stays open — no reconnection needed.
//
// Requires: OPENAI_API_KEY

use adk_realtime::config::{RealtimeConfig, SessionUpdateConfig, ToolDefinition};
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

    println!("=== Realtime Session Update — Mid-Session Persona Switch ===\n");

    // ── Phase 1: General assistant with weather tool ──
    let model = adk_realtime::openai::OpenAIRealtimeModel::new(&api_key, model_id);

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

    let runner = RealtimeRunner::builder()
        .model(Arc::new(model))
        .config(
            RealtimeConfig::default()
                .with_instruction("You are a helpful general assistant. Be concise — 1-2 sentences.")
                .with_voice("alloy")
                .with_modalities(vec!["text".to_string()]),
        )
        .tool(
            weather_tool,
            FnToolHandler::new(|call| {
                let city = call.arguments["city"].as_str().unwrap_or("unknown");
                println!("  🌤️ Weather tool called for: {}", city);
                Ok(json!({"city": city, "temp_f": 72, "condition": "sunny", "humidity": "45%"}))
            }),
        )
        .build()?;

    println!("📡 Connecting to OpenAI Realtime API...");
    runner.connect().await?;
    println!("✅ Connected — Phase 1: General Assistant\n");

    // Ask about weather
    let q1 = "What's the weather in Seattle?";
    println!("👤 User: {}\n", q1);
    runner.send_text(q1).await?;
    runner.create_response().await?;

    let response1 = collect_text_response(&runner).await;
    println!("🤖 Assistant: {}\n", response1);

    // ── Phase 2: Switch to travel agent mid-session ──
    println!("── Switching persona mid-session... ──\n");

    let flight_tool = ToolDefinition {
        name: "search_flights".into(),
        description: Some("Search for flights between cities".into()),
        parameters: Some(json!({
            "type": "object",
            "properties": {
                "from": { "type": "string" },
                "to": { "type": "string" }
            },
            "required": ["from", "to"]
        })),
    };

    let update = SessionUpdateConfig(
        RealtimeConfig::default()
            .with_instruction(
                "You are now a travel agent. Help users find flights and plan trips. \
                 Be enthusiastic and helpful. 1-2 sentences.",
            )
            .with_tools(vec![flight_tool]),
    );

    runner.update_session(update).await?;
    println!("✅ Session updated — Phase 2: Travel Agent\n");

    // Ask about flights (new persona + new tool)
    let q2 = "I need a flight from Seattle to Tokyo";
    println!("👤 User: {}\n", q2);
    runner.send_text(q2).await?;
    runner.create_response().await?;

    let response2 = collect_text_response(&runner).await;
    println!("🤖 Travel Agent: {}\n", response2);

    runner.close().await?;

    println!("=== Session Update Features ===");
    println!("• session.update switches persona + tools without reconnecting");
    println!("• WebSocket stays open — no latency penalty");
    println!("• Tools are swapped atomically (weather → flights)");
    println!("• Works with OpenAI Realtime; Gemini uses session resumption");
    Ok(())
}

async fn collect_text_response(runner: &RealtimeRunner) -> String {
    let mut text = String::new();
    let mut count = 0;
    while let Some(event) = runner.next_event().await {
        count += 1;
        match event {
            Ok(ServerEvent::TextDelta { delta, .. }) => text.push_str(&delta),
            Ok(ServerEvent::TranscriptDelta { delta, .. }) => text.push_str(&delta),
            Ok(ServerEvent::ResponseDone { .. }) => break,
            Ok(ServerEvent::FunctionCallDone { .. }) => {} // handled by runner
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
        if count > 200 { break; }
    }
    text
}
