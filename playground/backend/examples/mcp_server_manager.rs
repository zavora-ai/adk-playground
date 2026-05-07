use adk_core::{Content, SessionId, UserId};
use adk_rust::futures::StreamExt;
use adk_rust::prelude::*;
use adk_rust::session::{CreateRequest, SessionService};
use adk_tool::mcp::manager::{McpServerConfig, McpServerManager};
use std::collections::HashMap;
use std::sync::Arc;

// ── MCP Server Lifecycle Manager ──
// Demonstrates `McpServerManager` from v0.7:
//
// 1. Load MCP server configs from JSON (same format as mcp.json)
// 2. Start all servers with automatic process management
// 3. Query health status of managed servers
// 4. Aggregate tools from all servers into a unified toolset
// 5. Graceful shutdown with cleanup
//
// In production, McpServerManager handles:
// - Auto-restart on crash with configurable retry policy
// - Tool aggregation across multiple servers
// - Health monitoring and status reporting

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    println!("=== MCP Server Lifecycle Manager ===\n");

    // ── Part 1: Load config from JSON ──
    println!("── Part 1: Server Configuration from JSON ──\n");

    // Same format as .kiro/settings/mcp.json or VS Code mcp.json
    let json = r#"{
        "mcpServers": {
            "playwright": {
                "command": "npx",
                "args": ["--yes", "@playwright/mcp@latest"],
                "disabled": false,
                "autoApprove": ["browser_click", "browser_navigate"]
            },
            "filesystem": {
                "command": "npx",
                "args": ["--yes", "@modelcontextprotocol/server-filesystem", "/tmp"],
                "disabled": false,
                "autoApprove": ["read_file", "list_directory"]
            }
        }
    }"#;

    let configs: HashMap<String, McpServerConfig> =
        serde_json::from_str::<serde_json::Value>(json)?["mcpServers"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::from_value::<McpServerConfig>(v.clone()).unwrap()))
            .collect();

    println!("  📄 Loaded {} server configs from JSON:", configs.len());
    for (name, cfg) in &configs {
        println!("     • {} → {} {:?}", name, cfg.command, cfg.args);
    }

    // ── Part 2: Create and start the manager ──
    println!("\n── Part 2: Manager Lifecycle ──\n");

    let manager = McpServerManager::from_configs(configs);
    println!("  🚀 Starting MCP Server Manager...");
    println!("     (spawns child processes for each server)");

    // Note: In a real environment with npx installed, this would start the servers.
    // For the playground demo, we show the API patterns.
    match manager.start_all().await {
        Ok(()) => println!("  ✅ All servers started successfully"),
        Err(e) => println!("  ⚠️  Server start skipped (npx not available): {}", e),
    }

    // ── Part 3: Health monitoring ──
    println!("\n── Part 3: Health Monitoring ──\n");

    let statuses = manager.statuses().await;
    println!("  📊 Server Status:");
    for (name, status) in &statuses {
        println!("     {} — {:?}", name, status);
    }

    // ── Part 4: Tool aggregation ──
    println!("\n── Part 4: Tool Aggregation ──\n");
    println!("  McpServerManager aggregates tools from all running servers.");
    println!("  Use manager.tools(ctx) to get a unified tool list.");
    println!("  Each tool is tagged with its source server for routing.");

    // ── Part 5: Agent integration pattern ──
    println!("\n── Part 5: Agent with MCP Tools ──\n");

    let api_key = std::env::var("GOOGLE_API_KEY")?;
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-3.1-flash-lite-preview")?);

    // In production, you'd wire MCP tools into the agent:
    // let mcp_tools = manager.tools(&ctx).await?;
    // for tool in mcp_tools { builder = builder.tool(tool); }

    let agent = Arc::new(
        LlmAgentBuilder::new("mcp_agent")
            .instruction(
                "You are a helpful assistant with access to browser and filesystem tools \
                 via MCP servers. Describe what tools you would use to help the user.",
            )
            .model(model)
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
        intra_compaction_config: None,
        intra_compaction_summarizer: None,
    })?;

    let message = Content::new("user")
        .with_text("I need to read a config file at /tmp/app.toml and then open a webpage. How would you help?");

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

    // ── Part 6: Graceful shutdown ──
    println!("\n\n── Part 6: Graceful Shutdown ──\n");
    manager.stop_all().await;
    println!("  🛑 All MCP servers stopped");

    println!("\n=== Key Features ===");
    println!("• McpServerManager::from_configs() — load from mcp.json format");
    println!("• start_all() / stop_all() — lifecycle management");
    println!("• Auto-restart on crash with configurable retry policy");
    println!("• statuses() — real-time health of all managed servers");
    println!("• tools(ctx) — aggregate tools from all servers into one set");
    println!("• Same JSON format as VS Code / Kiro mcp.json configs");
    Ok(())
}
