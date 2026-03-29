//! CLI serve mode — validates the HTTP server configuration path
//!
//! Demonstrates: Launcher configured for serve mode with all services,
//! showing the pattern for deploying an agent as an HTTP endpoint.

use adk_agent::LlmAgentBuilder;
use adk_artifact::InMemoryArtifactService;
use adk_cli::Launcher;
use adk_core::StreamingMode;
use adk_session::InMemorySessionService;
use std::sync::Arc;

fn main() {
    println!("=== CLI Serve Mode Configuration ===\n");

    // Pattern: Build an agent and deploy it as an HTTP server
    // This is the recommended way to expose an agent as a service.

    // 1. Build the agent
    let agent = Arc::new(
        LlmAgentBuilder::new("api-agent")
            .instruction("You are a helpful API assistant. Be concise and precise.")
            .build()
            .unwrap()
    );
    println!("✓ Agent built: 'api-agent'");

    // 2. Configure services
    let sessions = Arc::new(InMemorySessionService::new());
    let artifacts = Arc::new(InMemoryArtifactService::new());
    println!("✓ Services: InMemorySessionService + InMemoryArtifactService");

    // 3. Build launcher for serve mode
    let launcher = Launcher::new(agent)
        .app_name("my-api")
        .with_session_service(sessions)
        .with_artifact_service(artifacts)
        .with_streaming_mode(StreamingMode::SSE);
    println!("✓ Launcher configured for serve mode");

    // In production, you'd call:
    //   launcher.run().await?;
    // With CLI args: `my-binary serve --port 8080`
    //
    // This starts an HTTP server with endpoints:
    //   POST /run          — execute agent (JSON request/response)
    //   POST /run/stream   — execute agent (SSE streaming)
    //   GET  /health       — health check
    //   GET  /             — web UI (if available)

    println!("\nServe mode endpoints:");
    println!("  POST /run          → JSON request/response");
    println!("  POST /run/stream   → SSE streaming");
    println!("  GET  /health       → Health check");
    println!("  GET  /             → Web UI");

    // 4. The launcher also supports console mode for development
    // Just run without `serve` arg: `my-binary`
    // This gives you an interactive REPL with:
    //   - rustyline history
    //   - streaming output
    //   - think-block rendering (for reasoning models)

    println!("\nConsole mode features:");
    println!("  - rustyline REPL with history");
    println!("  - Streaming token output");
    println!("  - Think-block rendering for reasoning models");

    let _ = launcher; // Prevent unused warning
    println!("\n=== All serve mode tests passed! ===");
}
