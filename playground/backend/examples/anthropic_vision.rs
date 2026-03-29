use adk_core::{Content, Part, SessionId, UserId};
use adk_rust::futures::StreamExt;
use adk_rust::prelude::*;
use adk_rust::session::{CreateRequest, SessionService};
use adk_tool::tool;
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

// ── Anthropic Vision Agent — Image Analysis with Tools ──
// Claude can see images via URL. This agent analyzes images and uses a tool
// to log structured observations. Demonstrates Part::FileData for image input
// combined with function tools in a Runner-based agent.

#[derive(Deserialize, JsonSchema)]
struct LogObservationArgs {
    /// Category of the observation (e.g., "animal", "scene", "object")
    category: String,
    /// Confidence level from 0.0 to 1.0
    confidence: f64,
    /// Detailed description of what was observed
    description: String,
}

/// Log a structured observation about an image.
#[tool]
async fn log_observation(args: LogObservationArgs) -> adk_tool::Result<serde_json::Value> {
    println!(
        "  📝 Observation logged: [{}] {:.0}% — {}",
        args.category,
        args.confidence * 100.0,
        args.description
    );
    Ok(serde_json::json!({
        "status": "logged",
        "category": args.category,
        "confidence": args.confidence
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
        LlmAgentBuilder::new("vision_analyst")
            .instruction(
                "You are an image analysis agent. When given an image, analyze it carefully \
                 and use the log_observation tool to record each distinct observation with \
                 a category and confidence score. After logging observations, provide a \
                 brief natural language summary.",
            )
            .model(model)
            .tool(Arc::new(LogObservation))
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

    println!("=== Anthropic Vision Agent — Image Analysis ===\n");

    // Send an image URL + text prompt as a multi-part message
    let image_url = "https://upload.wikimedia.org/wikipedia/commons/thumb/3/3a/Cat03.jpg/1200px-Cat03.jpg";
    println!("🖼️  Analyzing image: {}\n", image_url);

    let message = Content {
        role: "user".to_string(),
        parts: vec![
            Part::FileData {
                mime_type: "image/jpeg".to_string(),
                file_uri: image_url.to_string(),
            },
            Part::Text {
                text: "Analyze this image. Log each observation using the tool, then summarize.".to_string(),
            },
        ],
    };

    let mut stream = runner.run(uid, sid, message).await?;
    println!("🔍 Agent analyzing...\n");

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

    println!("\n\n=== Vision Capabilities ===");
    println!("• Part::FileData with image URL → Claude analyzes the image");
    println!("• Tools work alongside vision — structured extraction from images");
    println!("• Supports JPEG, PNG, GIF, WebP via URL or base64 inline data");
    Ok(())
}
