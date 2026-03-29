use adk_core::{SessionId, UserId};
use adk_rust::futures::StreamExt;
use adk_rust::prelude::*;
use adk_rust::session::{CreateRequest, SessionService};
use adk_tool::tool;
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

// ── Anthropic Structured Output — Typed JSON Extraction ──
// Agent extracts structured data from unstructured text using a tool.
// The tool schema forces Claude to return typed JSON matching our Rust struct.
// This pattern is ideal for data extraction, form filling, and API integration.

#[derive(Deserialize, JsonSchema, Debug)]
struct ExtractedContact {
    /// Full name of the person
    name: String,
    /// Email address if mentioned
    email: Option<String>,
    /// Company or organization
    company: Option<String>,
    /// Job title or role
    role: Option<String>,
    /// Key topics or interests mentioned
    topics: Vec<String>,
    /// Sentiment of the message (positive, neutral, negative)
    sentiment: String,
}

/// Extract structured contact information from text.
#[tool]
async fn extract_contact(args: ExtractedContact) -> adk_tool::Result<serde_json::Value> {
    println!("  📋 Extracted contact:");
    println!("     Name:      {}", args.name);
    if let Some(email) = &args.email {
        println!("     Email:     {}", email);
    }
    if let Some(company) = &args.company {
        println!("     Company:   {}", company);
    }
    if let Some(role) = &args.role {
        println!("     Role:      {}", role);
    }
    println!("     Topics:    {:?}", args.topics);
    println!("     Sentiment: {}", args.sentiment);
    Ok(serde_json::json!({
        "status": "extracted",
        "name": args.name,
        "email": args.email,
        "company": args.company,
        "role": args.role,
        "topics": args.topics,
        "sentiment": args.sentiment,
    }))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key =
        std::env::var("ANTHROPIC_API_KEY").expect("Set ANTHROPIC_API_KEY in your .env file");

    let model = Arc::new(AnthropicClient::new(
        AnthropicConfig::new(&api_key, "claude-sonnet-4-20250514")
            .with_max_tokens(1024),
    )?);

    let agent = Arc::new(
        LlmAgentBuilder::new("data_extractor")
            .instruction(
                "You are a data extraction agent. When given unstructured text, \
                 use the extract_contact tool to pull out structured information. \
                 Always call the tool — never respond with plain text. \
                 After extraction, confirm what you found in one sentence.",
            )
            .model(model)
            .tool(Arc::new(ExtractContact))
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
    })?;

    println!("=== Anthropic Structured Output — Contact Extraction ===\n");

    let emails = [
        (
            "Email 1 — Business inquiry",
            "Hi there! I'm Sarah Chen, lead engineer at Quantum Labs. \
             We're exploring Rust for our new distributed systems project. \
             I'd love to chat about your framework — reach me at sarah@quantumlabs.io. \
             Really impressed by the performance benchmarks!",
        ),
        (
            "Email 2 — Support request",
            "Hello, this is Marcus Rivera from DevOps at CloudScale Inc. \
             We've been having issues with memory leaks in production. \
             Our team is frustrated — we've spent weeks debugging this. \
             Can someone help? marcus.r@cloudscale.com",
        ),
    ];

    for (label, email_text) in &emails {
        println!("── {} ──\n", label);
        println!("📧 Input: \"{}\"\n", &email_text[..email_text.len().min(80)]);

        let message = Content::new("user").with_text(format!(
            "Extract contact information from this email:\n\n{}",
            email_text
        ));
        let mut stream = runner.run(uid.clone(), sid.clone(), message).await?;

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
        println!("\n");
    }

    println!("=== Pattern: Tool-as-Schema ===");
    println!("• Define a Rust struct with #[derive(Deserialize, JsonSchema)]");
    println!("• Use it as a #[tool] argument — Claude fills the schema");
    println!("• Typed extraction without prompt engineering for JSON format");
    Ok(())
}
