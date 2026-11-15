//! # Desktop Automation Agent — ADK-Rust + Gemini + computer-use-mcp
//!
//! Demonstrates using ADK-Rust with the `computer-use-mcp` server to control
//! a macOS desktop. MCP tool schemas use full JSON Schema (with fields like
//! `exclusiveMinimum`, tuple `items`, etc.) which are automatically handled
//! via Gemini's `parametersJsonSchema` field.
//!
//! ## Setup
//!
//! 1. Set `GOOGLE_API_KEY` (or `GEMINI_API_KEY`) environment variable
//! 2. Install computer-use-mcp: `npm install -g @zavora-ai/computer-use-mcp`
//! 3. Run: `cargo run --manifest-path examples/desktop_agent/Cargo.toml`
//!
//! Or pass a custom task:
//!
//! ```bash
//! cargo run --manifest-path examples/desktop_agent/Cargo.toml -- "Open Safari and go to google.com"
//! ```

use adk_agent::LlmAgentBuilder;
use adk_core::{Agent, Content, Part};
use adk_model::GeminiModel;
use adk_runner::Runner;
use adk_session::{InMemorySessionService, SessionService};
use adk_tool::mcp::manager::McpServerManager;
use futures::StreamExt;
use std::sync::Arc;
use std::time::Duration;

fn preview_chars(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let preview: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{preview}...")
    } else {
        preview
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,adk=debug".into()),
        )
        .init();

    println!("=== Desktop Automation Agent (ADK-Rust + Gemini) ===\n");

    // ── 1. Start the MCP server for desktop control ──────────────────
    let manager = McpServerManager::from_json(
        r#"{
            "mcpServers": {
                "desktop": {
                    "command": "npx",
                    "args": ["--yes", "--prefer-offline", "@zavora-ai/computer-use-mcp"],
                    "autoApprove": [
                        "screenshot", "zoom", "left_click", "right_click",
                        "middle_click", "double_click", "triple_click",
                        "mouse_move", "left_click_drag", "scroll",
                        "type", "key", "hold_key",
                        "get_frontmost_app", "list_windows", "list_running_apps",
                        "get_display_size", "cursor_position",
                        "read_clipboard", "write_clipboard",
                        "open_application", "activate_app",
                        "get_ui_tree", "find_element", "click_element",
                        "set_value", "press_button", "fill_form",
                        "snapshot", "wait", "run_script",
                        "select_menu_item", "list_menu_bar",
                        "get_tool_guide", "get_app_capabilities"
                    ]
                }
            }
        }"#,
    )?
    .with_health_check_interval(Duration::from_secs(60))
    .with_grace_period(Duration::from_secs(3));

    println!("Starting computer-use-mcp server...");
    let results = manager.start_all().await;
    for (name, result) in &results {
        match result {
            Ok(()) => println!("  ✓ {name} started"),
            Err(e) => {
                eprintln!("  ✗ {name} failed: {e}");
                return Err(format!("MCP server '{name}' failed to start: {e}").into());
            }
        }
    }

    // ── 2. Configure Gemini model ────────────────────────────────────
    let api_key = std::env::var("GOOGLE_API_KEY")
        .or_else(|_| std::env::var("GEMINI_API_KEY"))
        .expect("Set GOOGLE_API_KEY or GEMINI_API_KEY");

    let model = GeminiModel::new(&api_key, "gemini-2.5-flash")?;

    // ── 3. Build the agent ───────────────────────────────────────────
    let manager = Arc::new(manager);
    let agent = LlmAgentBuilder::new("desktop-agent")
        .description("Desktop automation agent using computer-use-mcp")
        .model(Arc::new(model))
        .instruction(
            "You are a desktop automation agent. You can control the computer \
             using the tools provided by the computer-use-mcp server.\n\n\
             Your approach:\n\
             1. Always take a screenshot first to understand the current state\n\
             2. Prefer scripting (run_script) for scriptable apps\n\
             3. Use accessibility (click_element, set_value) over coordinates\n\
             4. Fall back to coordinates (left_click, type) only when needed\n\
             5. Take a screenshot after each action to verify the result\n\n\
             You are running on macOS. Use bundle IDs for app targeting \
             (e.g., com.apple.Safari, com.apple.TextEdit).",
        )
        .toolset(Arc::clone(&manager) as Arc<dyn adk_core::Toolset>)
        .build()?;

    // ── 4. Create runner ─────────────────────────────────────────────
    let session_service = Arc::new(InMemorySessionService::new());

    let _session = session_service
        .create(adk_session::CreateRequest {
            app_name: "desktop-agent".to_string(),
            user_id: "user-1".to_string(),
            session_id: Some("session-1".to_string()),
            state: Default::default(),
        })
        .await?;

    let runner = Runner::builder()
        .app_name("desktop-agent")
        .agent(Arc::new(agent) as Arc<dyn Agent>)
        .session_service(session_service.clone())
        .build()?;

    // ── 5. Run the task ──────────────────────────────────────────────
    let task = std::env::args().nth(1).unwrap_or_else(|| {
        "Take a screenshot to see what's on screen, then list the running apps \
         and describe what you see."
            .to_string()
    });

    println!("\nTask: {task}\n");
    println!("{}", "─".repeat(60));

    let user_content = Content {
        role: "user".to_string(),
        parts: vec![Part::Text { text: task }],
    };

    let mut stream = runner.run_str("user-1", "session-1", user_content).await?;

    while let Some(event_result) = stream.next().await {
        match event_result {
            Ok(event) => {
                let author = &event.author;
                if let Some(content) = &event.llm_response.content {
                    for part in &content.parts {
                        match part {
                            Part::Text { text } => {
                                println!("[{author}] {text}");
                            }
                            Part::FunctionCall { name, args, .. } => {
                                let args_str =
                                    serde_json::to_string(args).unwrap_or_default();
                                let display = preview_chars(&args_str, 80);
                                println!("[{author}] → {name}({display})");
                            }
                            Part::FunctionResponse {
                                function_response, ..
                            } => {
                                let resp_str =
                                    serde_json::to_string(&function_response.response)
                                        .unwrap_or_default();
                                let display = preview_chars(&resp_str, 100);
                                println!(
                                    "[{author}] ← {}: {display}",
                                    function_response.name
                                );
                            }
                            _ => {}
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("Error: {e}");
                break;
            }
        }
    }

    println!("{}", "─".repeat(60));
    println!("\n✓ Done");

    // Gracefully shut down the MCP server
    manager.shutdown().await.ok();

    Ok(())
}
