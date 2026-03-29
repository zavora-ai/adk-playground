//! CLI Launcher — deploy agents as REPL or HTTP server
//!
//! Configures a Launcher with a real LLM agent, sessions, artifacts,
//! and streaming — then demonstrates the agent works by running it
//! through the Runner (Launcher uses Runner internally).

use adk_artifact::InMemoryArtifactService;
use adk_cli::Launcher;
use adk_core::StreamingMode;
use adk_core::{SessionId, UserId};
use adk_rust::futures::StreamExt;
use adk_rust::prelude::*;
use adk_rust::session::{CreateRequest, SessionService};
use std::collections::HashMap;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    println!("=== CLI Launcher — Agent Deployment ===\n");

    let api_key = std::env::var("GOOGLE_API_KEY")?;
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-3.1-flash-lite-preview")?);

    // ── 1. Build the agent ──
    let agent = Arc::new(
        LlmAgentBuilder::new("deploy-agent")
            .instruction(
                "You are a DevOps assistant that helps with deployment questions.\n\
                 You know about Docker, Kubernetes, CI/CD, and cloud platforms.\n\
                 Be concise and practical.",
            )
            .model(model)
            .build()?,
    );

    // ── 2. Configure services ──
    let sessions: Arc<dyn SessionService> = Arc::new(InMemorySessionService::new());
    let artifacts = Arc::new(InMemoryArtifactService::new());

    // ── 3. Show Launcher configuration ──
    // In production, Launcher::run() starts a REPL or HTTP server based on CLI args.
    // Here we show the config, then run the agent directly to demonstrate it works.
    let _launcher = Launcher::new(agent.clone())
        .app_name("devops-assistant")
        .with_session_service(sessions.clone())
        .with_artifact_service(artifacts)
        .with_streaming_mode(StreamingMode::SSE);

    println!("✓ Launcher configured:");
    println!("  app_name:  devops-assistant");
    println!("  sessions:  InMemorySessionService");
    println!("  artifacts: InMemoryArtifactService");
    println!("  streaming: SSE\n");

    println!("Deployment modes:");
    println!("  (no args)         → Interactive REPL");
    println!("  serve             → HTTP server on :3000");
    println!("  serve --port 8080 → Custom port\n");

    println!("Serve endpoints:");
    println!("  POST /run        → JSON request/response");
    println!("  POST /run/stream → SSE streaming");
    println!("  GET  /health     → Health check\n");

    // ── 4. Run the agent to prove it works ──
    println!("--- Demo: running the configured agent ---\n");

    sessions
        .create(CreateRequest {
            app_name: "devops-assistant".into(),
            user_id: "user".into(),
            session_id: Some("s1".into()),
            state: HashMap::new(),
        })
        .await?;

    let runner = Runner::new(RunnerConfig {
        app_name: "devops-assistant".into(),
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

    let query = "What's the simplest way to containerize a Rust web service with Docker? Give me a minimal Dockerfile.";
    println!("**User:** {}\n", query);
    print!("**Agent:** ");

    let message = Content::new("user").with_text(query);
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
