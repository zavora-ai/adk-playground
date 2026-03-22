use adk_rust::prelude::*;
use adk_rust::session::{SessionService, CreateRequest};
use adk_rust::futures::StreamExt;
use adk_core::{UserId, SessionId};
use std::collections::HashMap;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key = std::env::var("GOOGLE_API_KEY")?;

    let technical = Arc::new(
        LlmAgentBuilder::new("technical_analyst")
            .instruction("Analyze from a technical perspective. Be specific about implementation.")
            .model(Arc::new(GeminiModel::new(&api_key, "gemini-3.1-flash-lite-preview")?))
            .build()?
    ) as Arc<dyn Agent>;

    let business = Arc::new(
        LlmAgentBuilder::new("business_analyst")
            .instruction("Analyze from a business/market perspective. Focus on ROI and strategy.")
            .model(Arc::new(GeminiModel::new(&api_key, "gemini-3.1-flash-lite-preview")?))
            .build()?
    ) as Arc<dyn Agent>;

    let user_exp = Arc::new(
        LlmAgentBuilder::new("ux_analyst")
            .instruction("Analyze from a user experience perspective. Focus on usability.")
            .model(Arc::new(GeminiModel::new(&api_key, "gemini-3.1-flash-lite-preview")?))
            .build()?
    ) as Arc<dyn Agent>;

    // All three run concurrently, results merged
    let parallel = Arc::new(ParallelAgent::new(
        "multi_perspective_analysis",
        vec![technical, business, user_exp],
    ));

    let sessions = Arc::new(InMemorySessionService::new());
    sessions.create(CreateRequest {
        app_name: "playground".into(),
        user_id: "user".into(),
        session_id: Some("s1".into()),
        state: HashMap::new(),
    }).await?;

    let runner = Runner::new(RunnerConfig {
        app_name: "playground".into(),
        agent: parallel,
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

    println!("Running 3 analysts in parallel...\n");
    let message = Content::new("user")
        .with_text("Should a startup adopt WebAssembly for their web app?");
    let mut stream = runner.run(UserId::new("user")?, SessionId::new("s1")?, message).await?;

    while let Some(event) = stream.next().await {
        let event = event?;
        if let Some(content) = &event.llm_response.content {
            for part in &content.parts {
                if let Some(text) = part.text() { print!("{}", text); }
            }
        }
    }
    println!();
    Ok(())
}
