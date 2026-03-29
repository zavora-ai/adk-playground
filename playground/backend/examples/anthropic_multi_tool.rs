use adk_core::{SessionId, UserId};
use adk_rust::futures::StreamExt;
use adk_rust::prelude::*;
use adk_rust::session::{CreateRequest, SessionService};
use adk_tool::tool;
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

// ── Anthropic Multi-Tool Agent — Parallel Tool Calls ──
// Claude can call multiple tools in a single response. This agent has
// weather, calculator, and unit converter tools. When asked a complex
// question, Claude orchestrates multiple tool calls to build its answer.

#[derive(Deserialize, JsonSchema)]
struct WeatherArgs {
    /// City name to check weather for
    city: String,
}

/// Get current weather for a city.
#[tool]
async fn get_weather(args: WeatherArgs) -> adk_tool::Result<serde_json::Value> {
    let (temp_c, condition) = match args.city.to_lowercase().as_str() {
        "tokyo" => (22, "Partly cloudy"),
        "london" => (14, "Overcast"),
        "new york" => (18, "Clear"),
        "sydney" => (26, "Sunny"),
        "paris" => (16, "Light rain"),
        "berlin" => (12, "Foggy"),
        "mumbai" => (33, "Humid"),
        _ => (20, "Fair"),
    };
    println!("  🌤️ Weather for {}: {}°C, {}", args.city, temp_c, condition);
    Ok(serde_json::json!({
        "city": args.city,
        "temperature_celsius": temp_c,
        "condition": condition,
        "humidity": "65%",
        "wind_kmh": 12
    }))
}

#[derive(Deserialize, JsonSchema)]
struct CalcArgs {
    /// Mathematical expression to evaluate (e.g., "25 * 4")
    expression: String,
}

/// Evaluate a mathematical expression.
#[tool]
async fn calculate(args: CalcArgs) -> adk_tool::Result<serde_json::Value> {
    // Simple expression evaluator for demo
    let result = eval_simple(&args.expression);
    println!("  🧮 Calculate: {} = {}", args.expression, result);
    Ok(serde_json::json!({
        "expression": args.expression,
        "result": result
    }))
}

fn eval_simple(expr: &str) -> f64 {
    let parts: Vec<&str> = expr.split_whitespace().collect();
    if parts.len() == 3 {
        let a: f64 = parts[0].parse().unwrap_or(0.0);
        let b: f64 = parts[2].parse().unwrap_or(0.0);
        match parts[1] {
            "+" => a + b,
            "-" => a - b,
            "*" => a * b,
            "/" if b != 0.0 => a / b,
            _ => 0.0,
        }
    } else {
        expr.parse().unwrap_or(0.0)
    }
}

#[derive(Deserialize, JsonSchema)]
struct ConvertArgs {
    /// Value to convert
    value: f64,
    /// Source unit (celsius, fahrenheit, km, miles, kg, lbs)
    from_unit: String,
    /// Target unit
    to_unit: String,
}

/// Convert between units.
#[tool]
async fn convert_units(args: ConvertArgs) -> adk_tool::Result<serde_json::Value> {
    let result = match (args.from_unit.as_str(), args.to_unit.as_str()) {
        ("celsius", "fahrenheit") => args.value * 9.0 / 5.0 + 32.0,
        ("fahrenheit", "celsius") => (args.value - 32.0) * 5.0 / 9.0,
        ("km", "miles") => args.value * 0.621371,
        ("miles", "km") => args.value * 1.60934,
        ("kg", "lbs") => args.value * 2.20462,
        ("lbs", "kg") => args.value * 0.453592,
        _ => args.value,
    };
    println!("  🔄 Convert: {} {} → {:.1} {}", args.value, args.from_unit, result, args.to_unit);
    Ok(serde_json::json!({
        "original": args.value,
        "from": args.from_unit,
        "result": result,
        "to": args.to_unit
    }))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key =
        std::env::var("ANTHROPIC_API_KEY").expect("Set ANTHROPIC_API_KEY in your .env file");

    let model = Arc::new(AnthropicClient::new(
        AnthropicConfig::new(&api_key, "claude-sonnet-4-20250514")
            .with_max_tokens(1024),
    )?);

    let agent = Arc::new(
        LlmAgentBuilder::new("travel_assistant")
            .instruction(
                "You are a travel planning assistant with weather, calculator, and unit \
                 conversion tools. When comparing cities, check weather for ALL cities \
                 and convert temperatures so the user can compare easily. Use the \
                 calculator for any cost estimates. Be thorough — use multiple tools.",
            )
            .model(model)
            .tool(Arc::new(GetWeather))
            .tool(Arc::new(Calculate))
            .tool(Arc::new(ConvertUnits))
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

    println!("=== Anthropic Multi-Tool Agent — Travel Assistant ===\n");
    println!("Tools: get_weather, calculate, convert_units\n");

    let message = Content::new("user").with_text(
        "I'm deciding between Tokyo and Paris for a trip next week. \
         Compare the weather in both cities, convert the temperatures to Fahrenheit, \
         and calculate the price difference if Tokyo costs $2400 and Paris costs $1850.",
    );

    println!("👤 User: Compare Tokyo vs Paris for a trip\n");
    println!("🤖 Agent working...\n");

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

    println!("\n\n=== Multi-Tool Capabilities ===");
    println!("• Claude calls multiple tools in sequence to build comprehensive answers");
    println!("• Tool results feed back into the conversation for synthesis");
    println!("• #[tool] macro generates JSON Schema from Rust types automatically");
    Ok(())
}
