// Gemini Live — Voice Agent with Tool Calling
//
// Demonstrates Gemini Live API with function calling:
// The agent has weather and time tools. When asked a question,
// Gemini calls the tools mid-conversation and weaves results
// into its voice response — all over a single WebSocket.
//
// Key difference from OpenAI: Gemini Live uses Google AI Studio
// backend (GOOGLE_API_KEY) and supports native audio output.
//
// Requires: GOOGLE_API_KEY

use adk_realtime::config::{RealtimeConfig, ToolDefinition};
use adk_realtime::events::{ServerEvent, ToolResponse};
use adk_realtime::gemini::{GeminiLiveBackend, GeminiRealtimeModel};
use adk_realtime::{RealtimeModel, RealtimeSessionExt};
use serde_json::json;

fn get_weather(city: &str) -> String {
    let (temp, cond, humidity) = match city.to_lowercase().as_str() {
        "nairobi" => (22, "Partly cloudy", 65),
        "san francisco" => (15, "Foggy", 80),
        "tokyo" => (28, "Sunny", 55),
        "london" => (12, "Overcast", 75),
        "paris" => (17, "Light rain", 70),
        _ => (20, "Clear", 50),
    };
    json!({"city": city, "temperature_c": temp, "condition": cond, "humidity_pct": humidity}).to_string()
}

fn get_time(timezone: &str) -> String {
    let (offset, label) = match timezone.to_lowercase().as_str() {
        "eat" | "africa/nairobi" => ("+03:00", "East Africa Time"),
        "pst" | "america/los_angeles" => ("-08:00", "Pacific Standard Time"),
        "jst" | "asia/tokyo" => ("+09:00", "Japan Standard Time"),
        "gmt" | "europe/london" => ("+00:00", "Greenwich Mean Time"),
        "cet" | "europe/paris" => ("+01:00", "Central European Time"),
        _ => ("+00:00", "UTC"),
    };
    json!({"timezone": label, "utc_offset": offset, "current_time": "2026-03-29T14:30:00"}).to_string()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key = std::env::var("GOOGLE_API_KEY")?;

    println!("=== Gemini Live — Voice Agent with Tools ===\n");

    let backend = GeminiLiveBackend::studio(&api_key);
    let model = GeminiRealtimeModel::new(backend, "gemini-2.0-flash-live-001");

    let weather_tool = ToolDefinition::new("get_weather")
        .with_description("Get current weather for a city")
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "city": { "type": "string", "description": "City name" }
            },
            "required": ["city"]
        }));

    let time_tool = ToolDefinition::new("get_current_time")
        .with_description("Get current time in a timezone")
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "timezone": { "type": "string", "description": "IANA timezone or abbreviation (EAT, PST, JST)" }
            },
            "required": ["timezone"]
        }));

    let config = RealtimeConfig::default()
        .with_instruction(
            "You are a helpful voice assistant with weather and time tools. \
             When asked about weather or time, use the appropriate tool. \
             Keep responses concise and conversational — 2-3 sentences.",
        )
        .with_tool(weather_tool)
        .with_tool(time_tool);

    println!("📡 Connecting to Gemini Live API...");
    let session = model.connect(config).await?;
    println!("✅ Connected — Session: {}\n", session.session_id());
    println!("Tools: get_weather, get_current_time\n");

    // ── Turn 1: Ask about weather + time (should trigger both tools) ──
    let prompt = "What's the weather like in Nairobi right now, and what time is it there?";
    println!("👤 User: {}\n", prompt);
    session.send_text(prompt).await?;

    let mut full_text = String::new();
    let mut event_count = 0;

    loop {
        let event = match session.next_event().await {
            Some(Ok(ev)) => ev,
            Some(Err(e)) => { println!("❌ Error: {}", e); break; }
            None => break,
        };
        event_count += 1;

        match event {
            ServerEvent::FunctionCallDone { name, arguments, call_id, .. } => {
                println!("🔧 Tool: {}({})", name, &arguments[..arguments.len().min(60)]);
                let result = match name.as_str() {
                    "get_weather" => {
                        let args: serde_json::Value = serde_json::from_str(&arguments).unwrap_or(json!({}));
                        get_weather(args["city"].as_str().unwrap_or("unknown"))
                    }
                    "get_current_time" => {
                        let args: serde_json::Value = serde_json::from_str(&arguments).unwrap_or(json!({}));
                        get_time(args["timezone"].as_str().unwrap_or("UTC"))
                    }
                    _ => json!({"error": "unknown tool"}).to_string(),
                };
                println!("   → {}", &result[..result.len().min(80)]);
                session.send_tool_response(ToolResponse::from_string(call_id, result)).await?;
            }
            ServerEvent::TextDelta { delta, .. } => {
                print!("{}", delta);
                full_text.push_str(&delta);
            }
            ServerEvent::TranscriptDelta { delta, .. } => {
                print!("{}", delta);
                full_text.push_str(&delta);
            }
            ServerEvent::AudioDelta { delta, .. } => {
                // In a real app you'd play this audio
                let _ = delta.len();
            }
            ServerEvent::ResponseDone { .. } => {
                println!();
                break;
            }
            ServerEvent::Error { error, .. } => {
                println!("\n❌ {}: {}", error.error_type, error.message);
                break;
            }
            _ => {}
        }
        if event_count > 300 { break; }
    }

    println!("\n🤖 Response: {}", &full_text[..full_text.len().min(200)]);

    session.close().await?;

    println!("\n=== Gemini Live Features ===");
    println!("• GeminiLiveBackend::studio() — Google AI Studio with API key");
    println!("• ToolDefinition::new() — declarative tool schemas");
    println!("• FunctionCallDone → execute → send_tool_response loop");
    println!("• Native audio output (AudioDelta events)");
    println!("• WebSocket-based — low latency bidirectional streaming");
    Ok(())
}
