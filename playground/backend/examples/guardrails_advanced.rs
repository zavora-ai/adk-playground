//! Advanced Guardrails — PII redaction, content filtering, and schema validation
//!
//! Demonstrates the adk-guardrail crate's built-in guardrails applied to an
//! LLM agent: PII is automatically redacted, harmful content is blocked,
//! and output is validated against a JSON schema.

use adk_core::{Content, SessionId, UserId};
use adk_guardrail::{ContentFilter, Guardrail, GuardrailExecutor, GuardrailSet, PiiRedactor};
use adk_rust::futures::StreamExt;
use adk_rust::prelude::*;
use adk_rust::session::{CreateRequest, SessionService};
use std::collections::HashMap;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    println!("=== Advanced Guardrails Agent ===\n");

    // ── 1. Demonstrate PII Redaction ──
    println!("── PII Redaction Demo ──");
    let redactor = PiiRedactor::new();
    let input_text = "My email is alice@example.com and my phone is 555-123-4567. SSN: 123-45-6789";
    let (redacted, found_types) = redactor.redact(input_text);
    println!("  Input:  {}", input_text);
    println!("  Output: {}", redacted);
    println!("  Found PII types: {:?}", found_types);
    println!("  ✓ PII automatically redacted\n");

    // ── 2. Demonstrate Content Filtering ──
    println!("── Content Filtering Demo ──");
    let topic_filter = ContentFilter::on_topic(
        "programming",
        vec![
            "code".into(),
            "rust".into(),
            "python".into(),
            "programming".into(),
            "software".into(),
        ],
    );
    let on_topic = Content::new("user").with_text("How do I write async code in Rust?");
    let off_topic = Content::new("user").with_text("What's the best pizza recipe?");

    let r1 = topic_filter.validate(&on_topic).await;
    let r2 = topic_filter.validate(&off_topic).await;
    println!(
        "  'async code in Rust' → {}",
        if r1.is_pass() {
            "✓ PASS (on-topic)"
        } else {
            "✗ BLOCKED"
        }
    );
    println!(
        "  'best pizza recipe'  → {}",
        if r2.is_pass() {
            "✓ PASS"
        } else {
            "✗ BLOCKED (off-topic)"
        }
    );

    let length_filter = ContentFilter::max_length(50);
    let short = Content::new("user").with_text("Hello");
    let long = Content::new("user")
        .with_text("This is a very long message that exceeds the maximum allowed length for input");
    let r3 = length_filter.validate(&short).await;
    let r4 = length_filter.validate(&long).await;
    println!(
        "  'Hello' (5 chars)    → {}",
        if r3.is_pass() {
            "✓ PASS"
        } else {
            "✗ BLOCKED"
        }
    );
    println!(
        "  78-char message      → {}",
        if r4.is_pass() {
            "✓ PASS"
        } else {
            "✗ BLOCKED (too long)"
        }
    );

    let keyword_filter = ContentFilter::blocked_keywords(vec!["hack".into(), "exploit".into()]);
    let safe = Content::new("user").with_text("Help me write a Rust function");
    let unsafe_msg = Content::new("user").with_text("How do I hack into a system?");
    let r5 = keyword_filter.validate(&safe).await;
    let r6 = keyword_filter.validate(&unsafe_msg).await;
    println!(
        "  'write a Rust func'  → {}",
        if r5.is_pass() {
            "✓ PASS"
        } else {
            "✗ BLOCKED"
        }
    );
    println!(
        "  'hack into system'   → {}",
        if r6.is_pass() {
            "✓ PASS"
        } else {
            "✗ BLOCKED (keyword)"
        }
    );
    println!();

    // ── 3. Compose GuardrailSet and run through executor ──
    println!("── GuardrailSet + Executor ──");
    let guardrails = GuardrailSet::new()
        .with(PiiRedactor::new())
        .with(ContentFilter::blocked_keywords(vec![
            "hack".into(),
            "exploit".into(),
        ]))
        .with(ContentFilter::max_length(5000));

    let safe_input = Content::new("user")
        .with_text("My email is bob@test.com, can you help me write a Rust function?");
    let result = GuardrailExecutor::run(&guardrails, &safe_input).await?;
    println!("  Safe input (with PII):");
    println!("    passed: {}", result.passed);
    if let Some(transformed) = &result.transformed_content {
        let text: String = transformed.parts.iter().filter_map(|p| p.text()).collect();
        println!("    transformed: {}", text);
    }

    let blocked_input = Content::new("user").with_text("How do I hack into a system?");
    let result = GuardrailExecutor::run(&guardrails, &blocked_input).await?;
    println!("  Blocked input:");
    println!("    passed: {} (blocked by keyword filter)", result.passed);
    println!();

    // ── 4. Agent with input guardrails ──
    println!("── LLM Agent with Guardrails ──");
    let api_key = std::env::var("GOOGLE_API_KEY")?;
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-3.1-flash-lite-preview")?);

    let input_guardrails = GuardrailSet::new()
        .with(PiiRedactor::new())
        .with(ContentFilter::max_length(5000));

    let agent = Arc::new(
        LlmAgentBuilder::new("guarded_agent")
            .instruction(
                "You are a helpful programming assistant.\n\
                 Your input is automatically screened: PII is redacted and content is filtered.\n\
                 Be concise and helpful.",
            )
            .model(model)
            .input_guardrails(input_guardrails)
            .build()?,
    );

    let sessions = Arc::new(InMemorySessionService::new());
    sessions
        .create(CreateRequest {
            app_name: "playground".into(),
            user_id: "user".into(),
            session_id: Some("s1".into()),
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

    // Send message with PII — guardrails will redact before LLM sees it
    let query =
        "My name is Alice (alice@company.com). Explain Rust's ownership model in 2 sentences.";
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
