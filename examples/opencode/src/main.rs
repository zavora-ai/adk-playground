//! Streams one reply from an OpenCode Go model through `OpenCodeClient`.

use adk_core::{Content, Llm, LlmRequest, Part};
use adk_model::opencode::{OpenCodeClient, OpenCodeConfig, OpenCodeService};
use futures::TryStreamExt;
use std::io::Write;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    adk_core::ensure_crypto_provider();

    let api_key = std::env::var("OPENCODE_API_KEY")
        .map_err(|_| "OPENCODE_API_KEY must be set — see .env.example")?;

    println!("╔══════════════════════════════════════════════╗");
    println!("║  OpenCode Go Example — ADK-Rust              ║");
    println!("╚══════════════════════════════════════════════╝\n");

    // One session ID per conversation, reused for its main and auxiliary requests.
    let model = OpenCodeClient::new(
        OpenCodeConfig::new(OpenCodeService::Go, api_key, "deepseek-v4.1-flash")
            .with_user_agent("adk-opencode-example/1.0")
            .with_session_id("example-conversation-1"),
    )?;
    println!("  Model: {} via {:?}\n", model.name(), model.api());

    let request = LlmRequest::new(
        model.name(),
        vec![
            Content::new("user").with_text("Explain how to test a Rust function that reads a file"),
        ],
    );
    let mut stream = model.generate_content(request, true).await?;
    let mut streamed = false;
    while let Some(response) = stream.try_next().await? {
        let text: String = response
            .content
            .iter()
            .flat_map(|content| &content.parts)
            .filter_map(|part| match part {
                Part::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        // A closing non-partial response can repeat the streamed text, so it prints
        // only when no deltas arrived.
        if response.partial || !streamed {
            print!("{text}");
            std::io::stdout().flush()?;
        }
        streamed |= response.partial;
    }
    println!();
    Ok(())
}
