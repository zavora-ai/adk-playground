//! CLI Launcher modes — validates Launcher configuration and mode selection
//!
//! Demonstrates: Launcher::new, app_name, with_artifact_service,
//! with_session_service, with_streaming_mode, and mode configuration.

use adk_agent::LlmAgentBuilder;
use adk_artifact::InMemoryArtifactService;
use adk_cli::Launcher;
use adk_core::StreamingMode;
use adk_session::InMemorySessionService;
use std::sync::Arc;

fn main() {
    println!("=== CLI Launcher Modes ===\n");

    // 1. Minimal launcher — just an agent
    let agent = LlmAgentBuilder::new("assistant").build().unwrap();
    let _launcher = Launcher::new(Arc::new(agent));
    println!("✓ Minimal launcher: Launcher::new(agent)");

    // 2. Custom app name
    let agent = LlmAgentBuilder::new("assistant").build().unwrap();
    let _launcher = Launcher::new(Arc::new(agent))
        .app_name("my-cool-app");
    println!("✓ Custom app name: .app_name(\"my-cool-app\")");

    // 3. With artifact service
    let agent = LlmAgentBuilder::new("assistant").build().unwrap();
    let artifacts = Arc::new(InMemoryArtifactService::new());
    let _launcher = Launcher::new(Arc::new(agent))
        .with_artifact_service(artifacts);
    println!("✓ With artifacts: .with_artifact_service(InMemoryArtifactService)");

    // 4. With session service
    let agent = LlmAgentBuilder::new("assistant").build().unwrap();
    let sessions = Arc::new(InMemorySessionService::new());
    let _launcher = Launcher::new(Arc::new(agent))
        .with_session_service(sessions);
    println!("✓ With sessions: .with_session_service(InMemorySessionService)");

    // 5. Streaming mode selection
    let agent = LlmAgentBuilder::new("assistant").build().unwrap();
    let _launcher = Launcher::new(Arc::new(agent))
        .with_streaming_mode(StreamingMode::SSE);
    println!("✓ SSE streaming: .with_streaming_mode(StreamingMode::SSE)");

    let agent = LlmAgentBuilder::new("assistant").build().unwrap();
    let _launcher = Launcher::new(Arc::new(agent))
        .with_streaming_mode(StreamingMode::None);
    println!("✓ No streaming: .with_streaming_mode(StreamingMode::None)");

    // 6. Full configuration
    let agent = LlmAgentBuilder::new("full-agent").build().unwrap();
    let _launcher = Launcher::new(Arc::new(agent))
        .app_name("production-app")
        .with_artifact_service(Arc::new(InMemoryArtifactService::new()))
        .with_session_service(Arc::new(InMemorySessionService::new()))
        .with_streaming_mode(StreamingMode::SSE);
    println!("✓ Full config: app_name + artifacts + sessions + SSE");

    // Note: .run().await would start the CLI based on command-line args:
    //   - No args → interactive REPL mode
    //   - `serve` → HTTP server mode
    //   - `serve --port 8080` → HTTP server on custom port
    println!("\nLauncher modes (when .run().await is called):");
    println!("  (no args)           → Interactive REPL with rustyline");
    println!("  serve               → HTTP server with web UI");
    println!("  serve --port 8080   → HTTP server on custom port");

    println!("\n=== All launcher mode tests passed! ===");
}
