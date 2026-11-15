//! Enterprise Hello World
//!
//! The simplest possible ADK Enterprise session.
//! Creates an agent, starts a session, streams the response to a message,
//! and cleans up all resources.
//!
//! # Usage
//!
//! ```bash
//! export ADK_API_KEY=adk_live_...
//! cargo run -p enterprise-hello
//! ```

use adk_enterprise::{
    ContentBlock, CreateAgentParams, EnterpriseClient, SessionEvent,
};
use futures::StreamExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    eprintln!("=== ADK Enterprise: Hello World ===\n");

    // 1. Create client from ADK_API_KEY env var (graceful error if missing)
    let client = match EnterpriseClient::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("ERROR: {e}");
            eprintln!();
            eprintln!("To run this example, set your API key:");
            eprintln!("  export ADK_API_KEY=adk_live_...");
            eprintln!();
            eprintln!("You can get a key at https://enterprise.adk-rust.com");
            std::process::exit(1);
        }
    };
    eprintln!("✓ Client created");

    // 2. Create an agent with gemini-2.5-flash
    let agent = client
        .create_agent(CreateAgentParams {
            name: "Hello Agent".into(),
            model: "gemini-2.5-flash".into(),
            system: Some("You are brief and friendly.".into()),
            ..Default::default()
        })
        .await?;
    eprintln!("✓ Agent created: {}", agent.id);

    // 3. Create a session
    let session = client.create_session(&agent.id, None).await?;
    eprintln!("✓ Session created: {}", session.id);

    // 4. Open stream BEFORE sending (required ordering)
    let mut stream = client.stream_events(&session.id).await?;
    eprintln!("✓ Stream opened\n");

    // 5. Send a message
    client.send_message(&session.id, "What is 2+2?").await?;
    eprintln!("→ Sent: \"What is 2+2?\"\n");

    // 6. Process events — print text content, break on idle
    eprintln!("← Agent response:");
    while let Some(event) = stream.next().await {
        match event? {
            SessionEvent::Message { content, .. } => {
                for block in content {
                    if let ContentBlock::Text { text } = block {
                        println!("{text}");
                    }
                }
            }
            SessionEvent::StatusIdle { .. } => {
                eprintln!("\n✓ Session idle");
                break;
            }
            SessionEvent::Error { message, .. } => {
                eprintln!("\n✗ Error: {message}");
                break;
            }
            _ => {}
        }
    }

    // 7. Archive the session
    eprintln!("\nCleaning up...");
    client.archive_session(&session.id).await?;
    eprintln!("  ✓ Session archived");

    // 8. Delete the agent
    client.delete_agent(&agent.id).await?;
    eprintln!("  ✓ Agent deleted");

    eprintln!("\n=== Done ===");
    Ok(())
}
