//! # Enterprise Custom Tool Example
//!
//! Demonstrates the full custom tool round-trip pattern with `adk-enterprise`:
//!
//! 1. Create an agent with a custom `get_weather` tool
//! 2. Create a session
//! 3. Open an SSE event stream
//! 4. Send "What's the weather in Tokyo?"
//! 5. Handle `CustomToolUse` → execute locally → send result back
//! 6. Print the agent's final response
//! 7. Clean up (archive session, delete agent)

use adk_enterprise::{ContentBlock, CreateAgentParams, EnterpriseClient, SessionEvent, ToolConfig};
use futures::StreamExt;
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("=== ADK Enterprise: Custom Tool Example ===\n");

    // ─── 1. Create client from environment ───────────────────────────
    let client = EnterpriseClient::from_env().map_err(|e| {
        eprintln!("Error: {e}");
        eprintln!("Set the ADK_API_KEY environment variable to your Enterprise API key.");
        e
    })?;

    println!("[1/7] Client created successfully.");

    // ─── 2. Create agent with a custom tool ──────────────────────────
    let agent = client
        .create_agent(CreateAgentParams {
            name: "Weather Agent".into(),
            model: "gemini-2.5-flash".into(),
            system: Some(
                "You are a helpful weather assistant. Use the get_weather tool to look up weather information.".into(),
            ),
            tools: vec![ToolConfig::custom(
                "get_weather",
                "Get the current weather for a city",
                json!({
                    "type": "object",
                    "properties": {
                        "city": {
                            "type": "string",
                            "description": "The city name to get weather for"
                        }
                    },
                    "required": ["city"]
                }),
            )],
            ..Default::default()
        })
        .await?;

    println!("[2/7] Agent created: id={}", agent.id);

    // ─── 3. Create a session ─────────────────────────────────────────
    let session = client.create_session(&agent.id, None).await?;
    println!("[3/7] Session created: id={}", session.id);

    // ─── 4. Open SSE event stream ────────────────────────────────────
    let mut stream = client.stream_events(&session.id).await?;
    println!("[4/7] SSE stream opened.");

    // ─── 5. Send a message that will trigger the custom tool ─────────
    client.send_message(&session.id, "What's the weather in Tokyo?").await?;
    println!("[5/7] Message sent: \"What's the weather in Tokyo?\"\n");

    // ─── 6. Event loop: handle custom tool use and print response ────
    println!("--- Streaming events ---");
    while let Some(event) = stream.next().await {
        match event? {
            SessionEvent::CustomToolUse { custom_tool_use_id, name, input, .. } => {
                println!("  [CustomToolUse] tool={name}, input={input}");

                // Execute the tool locally
                let result = execute_custom_tool(&name, &input);
                println!("  [ToolResult] {result}");

                // Send the result back to the agent as a content block
                client
                    .custom_tool_result(
                        &session.id,
                        &custom_tool_use_id,
                        vec![ContentBlock::text(result)],
                    )
                    .await?;
            }
            SessionEvent::Message { content, .. } => {
                for block in &content {
                    if let ContentBlock::Text { text } = block {
                        println!("  [Agent] {text}");
                    }
                }
            }
            SessionEvent::StatusIdle { stop_reason, .. } => {
                println!("  [StatusIdle] stop_reason={stop_reason:?}");
                break;
            }
            SessionEvent::StatusRunning { .. } => {
                println!("  [StatusRunning]");
            }
            SessionEvent::Error { message, code, .. } => {
                println!("  [Error] code={code:?} message={message}");
                break;
            }
            _ => {}
        }
    }
    println!("--- Stream ended ---\n");

    // ─── 7. Clean up ─────────────────────────────────────────────────
    client.archive_session(&session.id).await?;
    println!("[6/7] Session archived.");

    client.delete_agent(&agent.id).await?;
    println!("[7/7] Agent deleted.");

    println!("\nDone!");
    Ok(())
}

/// Execute a custom tool locally and return the result string.
fn execute_custom_tool(name: &str, input: &serde_json::Value) -> String {
    match name {
        "get_weather" => {
            let city = input["city"].as_str().unwrap_or("unknown");
            format!("22°C, sunny in {city}")
        }
        _ => format!("Unknown tool: {name}"),
    }
}
