use adk_rust::prelude::*;
use adk_tool::tool;
use adk_rust::session::{SessionService, CreateRequest};
use adk_rust::futures::StreamExt;
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Deserialize, JsonSchema)]
struct WeatherArgs {
    /// The city to look up
    city: String,
}

/// Get current weather for a city.
#[tool]
async fn get_weather(args: WeatherArgs) -> adk_tool::Result<serde_json::Value> {
    Ok(serde_json::json!({
        "city": args.city,
        "temp_c": 22,
        "condition": "Sunny",
        "humidity": "45%"
    }))
}

#[derive(Deserialize, JsonSchema)]
struct TimeArgs {
    /// The timezone to look up (e.g. "UTC", "JST", "EST")
    timezone: String,
}

/// Get current time for a timezone.
#[tool]
async fn get_time(args: TimeArgs) -> adk_tool::Result<serde_json::Value> {
    Ok(serde_json::json!({
        "timezone": args.timezone,
        "time": "14:30",
        "date": "2026-03-17"
    }))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key = std::env::var("GOOGLE_API_KEY")?;
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-2.5-flash")?);

    let agent = Arc::new(
        LlmAgentBuilder::new("weather_time_agent")
            .instruction(
                "You help users check weather and time. Use get_weather for weather \
                 and get_time for time queries. Be concise."
            )
            .model(model)
            .tool(Arc::new(GetWeather))
            .tool(Arc::new(GetTime))
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

    let message = Content::new("user")
        .with_text("What's the weather in Tokyo and what time is it there?");
    let mut stream = runner.run("user".into(), "s1".into(), message).await?;

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
