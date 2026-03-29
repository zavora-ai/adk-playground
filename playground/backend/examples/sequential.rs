use adk_core::{SessionId, UserId};
use adk_rust::futures::StreamExt;
use adk_rust::prelude::*;
use adk_rust::session::{CreateRequest, SessionService};
use std::collections::HashMap;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key = std::env::var("GOOGLE_API_KEY")?;

    let researcher = Arc::new(
        LlmAgentBuilder::new("researcher")
            .instruction("Research the given topic. Identify 3 key points with evidence.")
            .model(Arc::new(GeminiModel::new(
                &api_key,
                "gemini-3.1-flash-lite-preview",
            )?))
            .build()?,
    ) as Arc<dyn Agent>;

    let writer = Arc::new(
        LlmAgentBuilder::new("writer")
            .instruction("Take the research and write a polished 2-paragraph summary.")
            .model(Arc::new(GeminiModel::new(
                &api_key,
                "gemini-3.1-flash-lite-preview",
            )?))
            .build()?,
    ) as Arc<dyn Agent>;

    let editor = Arc::new(
        LlmAgentBuilder::new("editor")
            .instruction("Edit for clarity and conciseness. Fix any issues. Output final version.")
            .model(Arc::new(GeminiModel::new(
                &api_key,
                "gemini-3.1-flash-lite-preview",
            )?))
            .build()?,
    ) as Arc<dyn Agent>;

    // Chain: researcher → writer → editor
    let pipeline = Arc::new(SequentialAgent::new(
        "research_pipeline",
        vec![researcher, writer, editor],
    ));

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
        agent: pipeline,
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

    println!("Running 3-stage pipeline: researcher → writer → editor\n");
    let message = Content::new("user").with_text("The impact of Rust on systems programming");
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
