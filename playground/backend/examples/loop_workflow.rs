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
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-3.1-flash-lite-preview")?);

    let refiner = Arc::new(
        LlmAgentBuilder::new("refiner")
            .instruction(
                "Refine the given content to make it better. \
                 If the content is good enough (clear, concise, well-structured), \
                 call the exit_loop tool with the final result. \
                 Otherwise, provide an improved version.",
            )
            .model(model)
            .tool(Arc::new(ExitLoopTool::new()))
            .build()?,
    ) as Arc<dyn Agent>;

    // Loop up to 5 iterations, exit early when quality is sufficient
    let loop_agent =
        Arc::new(LoopAgent::new("refinement_loop", vec![refiner]).with_max_iterations(5));

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
        agent: loop_agent,
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

    println!("Running iterative refinement loop (max 5 iterations)...\n");
    let message = Content::new("user").with_text("Write a haiku about programming in Rust");
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
