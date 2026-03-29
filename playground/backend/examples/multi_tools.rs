use adk_core::{SessionId, UserId};
use adk_rust::futures::StreamExt;
use adk_rust::prelude::*;
use adk_rust::session::{CreateRequest, SessionService};
use adk_tool::tool;
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Deserialize, JsonSchema)]
struct WeatherArgs {
    /// The city to look up
    city: String,
}

/// Get the current weather for a city.
#[tool]
async fn get_weather(args: WeatherArgs) -> adk_tool::Result<serde_json::Value> {
    Ok(serde_json::json!({ "city": args.city, "temperature": "22°C", "condition": "sunny" }))
}

#[derive(Deserialize, JsonSchema)]
struct CalcArgs {
    /// First number
    a: f64,
    /// Second number
    b: f64,
    /// Operation: add, subtract, multiply, divide
    operation: String,
}

/// Perform arithmetic on two numbers.
#[tool]
async fn calculate(args: CalcArgs) -> adk_tool::Result<serde_json::Value> {
    let result = match args.operation.as_str() {
        "add" => args.a + args.b,
        "subtract" => args.a - args.b,
        "multiply" => args.a * args.b,
        "divide" if args.b != 0.0 => args.a / args.b,
        _ => 0.0,
    };
    Ok(serde_json::json!({ "result": result }))
}

#[derive(Deserialize, JsonSchema)]
struct ConvertArgs {
    /// The numeric value to convert
    value: f64,
    /// Source unit (celsius, fahrenheit, km, miles, kg, lbs)
    from: String,
    /// Target unit
    to: String,
}

/// Convert between units (temperature, distance, weight).
#[tool]
async fn convert_units(args: ConvertArgs) -> adk_tool::Result<serde_json::Value> {
    let result = match (args.from.as_str(), args.to.as_str()) {
        ("celsius", "fahrenheit") => args.value * 9.0 / 5.0 + 32.0,
        ("fahrenheit", "celsius") => (args.value - 32.0) * 5.0 / 9.0,
        ("km", "miles") => args.value * 0.621371,
        ("miles", "km") => args.value / 0.621371,
        ("kg", "lbs") => args.value * 2.20462,
        ("lbs", "kg") => args.value / 2.20462,
        _ => args.value,
    };
    Ok(
        serde_json::json!({ "value": args.value, "from": args.from, "to": args.to, "result": result }),
    )
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key = std::env::var("GOOGLE_API_KEY")?;
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-3.1-flash-lite-preview")?);

    let agent = Arc::new(
        LlmAgentBuilder::new("multi_tool_agent")
            .instruction(
                "You are a helpful assistant with multiple tools:\n\
                 - get_weather: weather lookups\n\
                 - calculate: arithmetic operations\n\
                 - convert_units: unit conversions (celsius/fahrenheit, km/miles, kg/lbs)\n\
                 Use the appropriate tool for each part of the user's request.",
            )
            .model(model)
            .tool(Arc::new(GetWeather))
            .tool(Arc::new(Calculate))
            .tool(Arc::new(ConvertUnits))
            .build()?,
    );

    let sessions = Arc::new(InMemorySessionService::new());
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

    let message = Content::new("user").with_text(
        "What's the weather in Tokyo? Convert 22°C to Fahrenheit. Also what's 15% of 250?",
    );
    let mut stream = runner
        .run(UserId::new("user")?, SessionId::new("s1")?, message)
        .await?;

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
    println!();
    Ok(())
}
