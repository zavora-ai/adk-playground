use adk_core::{Content, SessionId, UserId};
use adk_rust::futures::StreamExt;
use adk_rust::prelude::*;
use adk_rust::session::{CreateRequest, SessionService};
use adk_tool::tool;
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

// ── ACP Server — Agent Client Protocol (v0.8.1) ──
// Demonstrates exposing an ADK-Rust agent as an ACP-compatible server
// that IDEs (Kiro, VS Code, Claude Code, etc.) can connect to.
//
// ACP Protocol flow:
//   Client (IDE) ──stdin──► ACP Server ──► ADK Agent
//                ◄─stdout──             ◄──
//
// Message sequence:
//   1. initialize → capabilities
//   2. session/create → session ID
//   3. session/prompt → agent response (streaming notifications)
//   4. session/close → cleanup
//
// This example shows the agent that would back an ACP server,
// with file-system tools typical of a coding assistant.

#[derive(Deserialize, JsonSchema)]
struct ReadFileArgs {
    /// Path to the file to read
    path: String,
}

/// Read the contents of a file at the given path.
#[tool]
async fn read_file(args: ReadFileArgs) -> adk_tool::Result<serde_json::Value> {
    println!("  📄 read_file: {}", args.path);
    // Simulated file content for demo
    let content = match args.path.as_str() {
        "src/main.rs" => "fn main() {\n    println!(\"Hello, world!\");\n}",
        "Cargo.toml" => "[package]\nname = \"my-app\"\nversion = \"0.1.0\"\nedition = \"2024\"",
        _ => "// File not found in demo",
    };
    Ok(serde_json::json!({
        "path": args.path,
        "content": content,
        "lines": content.lines().count()
    }))
}

#[derive(Deserialize, JsonSchema)]
struct WriteFileArgs {
    /// Path to write the file
    path: String,
    /// Content to write
    content: String,
}

/// Write content to a file, creating it if it doesn't exist.
#[tool]
async fn write_file(args: WriteFileArgs) -> adk_tool::Result<serde_json::Value> {
    println!("  ✏️  write_file: {} ({} bytes)", args.path, args.content.len());
    Ok(serde_json::json!({
        "path": args.path,
        "bytes_written": args.content.len(),
        "success": true
    }))
}

#[derive(Deserialize, JsonSchema)]
struct ListFilesArgs {
    /// Directory path to list
    directory: String,
}

/// List files in a directory.
#[tool]
async fn list_files(args: ListFilesArgs) -> adk_tool::Result<serde_json::Value> {
    println!("  📁 list_files: {}", args.directory);
    Ok(serde_json::json!({
        "directory": args.directory,
        "files": ["src/main.rs", "src/lib.rs", "Cargo.toml", "README.md"],
        "count": 4
    }))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    println!("=== ACP Server — Agent Client Protocol (v0.8.1) ===\n");

    // ── Part 1: ACP Protocol Overview ──
    println!("── Part 1: ACP Protocol Overview ──\n");
    println!("  The Agent Client Protocol (ACP) connects IDEs to AI agents:");
    println!("  • Kiro, VS Code, Claude Code, Codex → any ACP-compatible agent");
    println!("  • Newline-delimited JSON over stdio (or HTTP/SSE for remote)");
    println!("  • Session-based: create → prompt → close lifecycle");
    println!("  • Permission bridge: agent can request user approval mid-flow");
    println!();
    println!("  ACP Server Config:");
    println!("  ┌─────────────────────────────────────────────────────┐");
    println!("  │ AcpServerConfig {{                                    │");
    println!("  │   name: \"coding-assistant\",                          │");
    println!("  │   version: \"1.0.0\",                                  │");
    println!("  │   transport: TransportConfig::Stdio,                 │");
    println!("  │   auto_approve_permissions: false,                   │");
    println!("  │ }}                                                     │");
    println!("  └─────────────────────────────────────────────────────┘");

    // ── Part 2: Build the coding assistant agent ──
    println!("\n── Part 2: Coding Assistant Agent ──\n");

    let api_key = std::env::var("GOOGLE_API_KEY")?;
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-3.1-flash-lite-preview")?);

    let agent = Arc::new(
        LlmAgentBuilder::new("coding_assistant")
            .instruction(
                "You are a coding assistant connected via ACP to an IDE. \
                 You can read files, write files, and list directories. \
                 Help the user understand and modify their code. Be concise.",
            )
            .model(model)
            .tool(Arc::new(ReadFile))
            .tool(Arc::new(WriteFile))
            .tool(Arc::new(ListFiles))
            .build()?,
    );

    println!("  🤖 Agent: coding_assistant");
    println!("     Tools: read_file, write_file, list_files");
    println!("     Model: gemini-3.1-flash-lite-preview");

    // ── Part 3: Simulate ACP session ──
    println!("\n── Part 3: Simulated ACP Session ──\n");

    let sessions = Arc::new(InMemorySessionService::new());
    let uid = UserId::new("ide-user")?;
    let sid = SessionId::new("acp-session-1")?;
    sessions
        .create(CreateRequest {
            app_name: "acp-server".into(),
            user_id: uid.to_string(),
            session_id: Some(sid.to_string()),
            state: HashMap::new(),
        })
        .await?;

    let runner = Runner::new(RunnerConfig {
        app_name: "acp-server".into(),
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
        intra_compaction_config: None,
        intra_compaction_summarizer: None,
    })?;

    // Simulate an IDE prompt
    println!("  → session/prompt: \"List the files and read main.rs\"\n");

    let message = Content::new("user")
        .with_text("List the files in the project and then read src/main.rs. Summarize what the project does.");

    print!("  🤖 ");
    let mut stream = runner.run(uid, sid, message).await?;
    while let Some(event) = stream.next().await {
        let event = event?;
        if let Some(content) = event.content() {
            for part in &content.parts {
                if let Some(text) = part.text() {
                    print!("{}", text);
                }
            }
        }
    }

    println!("\n\n=== Key Features ===");
    println!("• adk-acp crate — expose any ADK agent as an ACP server");
    println!("• AcpServer::run(config) → AcpServerHandle (programmatic API)");
    println!("• StdioTransport — newline-delimited JSON over stdin/stdout");
    println!("• PermissionBridge — bidirectional ADK ↔ ACP permission flow");
    println!("• ResponseStreamer — ADK Events → ACP SessionNotifications");
    println!("• Works with Kiro, VS Code, Claude Code, and any ACP client");
    println!("• Feature-gated: adk-rust = {{ features = [\"acp\"] }}");
    Ok(())
}
