use adk_core::{SessionId, UserId};
use adk_rust::futures::StreamExt;
use adk_rust::prelude::*;
use adk_rust::session::{CreateRequest, SessionService};
use adk_tool::tool;
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

// ── Agentic Web Protocol (AWP) — Agent Discovery & Capabilities ──
// Demonstrates the AWP protocol from v0.7:
//
// AWP enables agents to discover and interact with other agents on the web.
// Think of it as DNS + REST for AI agents — each agent publishes a manifest
// describing its capabilities, trust level, and rate limits.
//
// This example shows:
// 1. Building an agent that exposes AWP-compatible capabilities
// 2. Capability manifests with trust levels and rate limiting
// 3. Agent-to-agent discovery and invocation patterns
//
// In production, AWP agents serve at /.well-known/awp.json for discovery.

#[derive(Deserialize, JsonSchema)]
struct TranslateArgs {
    /// Text to translate
    text: String,
    /// Target language code (e.g., "es", "fr", "de", "ja")
    target_lang: String,
}

/// Translate text to a target language.
#[tool]
async fn translate(args: TranslateArgs) -> adk_tool::Result<serde_json::Value> {
    println!("  🌐 Translating to {}: \"{}\"", args.target_lang, args.text);
    let translated = match args.target_lang.as_str() {
        "es" => format!("[ES] {}", args.text),
        "fr" => format!("[FR] {}", args.text),
        "de" => format!("[DE] {}", args.text),
        "ja" => format!("[JA] {}", args.text),
        _ => format!("[{}] {}", args.target_lang.to_uppercase(), args.text),
    };
    Ok(serde_json::json!({
        "original": args.text,
        "translated": translated,
        "target_lang": args.target_lang,
        "confidence": 0.95
    }))
}

#[derive(Deserialize, JsonSchema)]
struct SentimentArgs {
    /// Text to analyze for sentiment
    text: String,
}

/// Analyze the sentiment of text.
#[tool]
async fn analyze_sentiment(args: SentimentArgs) -> adk_tool::Result<serde_json::Value> {
    println!("  💭 Analyzing sentiment: \"{}\"", &args.text[..args.text.len().min(50)]);
    Ok(serde_json::json!({
        "text": args.text,
        "sentiment": "positive",
        "score": 0.87,
        "emotions": ["enthusiasm", "curiosity"]
    }))
}

#[derive(Deserialize, JsonSchema)]
struct SummarizeArgs {
    /// Text to summarize
    text: String,
    /// Maximum sentences in summary
    max_sentences: Option<u32>,
}

/// Summarize text to key points.
#[tool]
async fn summarize_text(args: SummarizeArgs) -> adk_tool::Result<serde_json::Value> {
    let max = args.max_sentences.unwrap_or(3);
    println!("  📋 Summarizing ({} sentence max)", max);
    Ok(serde_json::json!({
        "summary": format!("Key points from the text (max {} sentences)", max),
        "sentence_count": max.min(2),
        "compression_ratio": 0.3
    }))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    println!("=== Agentic Web Protocol (AWP) — Discovery & Capabilities ===\n");

    // ── Part 1: Define AWP capability manifest ──
    println!("── Part 1: AWP Capability Manifest ──\n");

    // In production, this would be served at /.well-known/awp.json
    let manifest = serde_json::json!({
        "name": "language-services",
        "version": "1.0.0",
        "description": "Multi-language text processing agent",
        "capabilities": [
            {
                "name": "translate",
                "description": "Translate text between languages",
                "input_schema": {"type": "object", "properties": {"text": {"type": "string"}, "target_lang": {"type": "string"}}},
                "trust_level": "public",
                "rate_limit": {"requests_per_minute": 60}
            },
            {
                "name": "sentiment",
                "description": "Analyze text sentiment and emotions",
                "input_schema": {"type": "object", "properties": {"text": {"type": "string"}}},
                "trust_level": "authenticated",
                "rate_limit": {"requests_per_minute": 30}
            },
            {
                "name": "summarize",
                "description": "Summarize text to key points",
                "input_schema": {"type": "object", "properties": {"text": {"type": "string"}, "max_sentences": {"type": "integer"}}},
                "trust_level": "public",
                "rate_limit": {"requests_per_minute": 45}
            }
        ],
        "health_endpoint": "/health",
        "events_endpoint": "/events",
        "consent_required": false
    });

    println!("  📄 AWP Manifest: language-services v1.0.0");
    println!("     Capabilities:");
    if let Some(caps) = manifest["capabilities"].as_array() {
        for cap in caps {
            println!("       • {} — trust={}, rate={}rpm",
                cap["name"].as_str().unwrap_or("?"),
                cap["trust_level"].as_str().unwrap_or("?"),
                cap["rate_limit"]["requests_per_minute"]);
        }
    }
    println!("\n     Discovery: GET /.well-known/awp.json");
    println!("     Health:    GET /health");
    println!("     Events:    SSE /events");

    // ── Part 2: Build the AWP-capable agent ──
    println!("\n── Part 2: AWP Agent with Tools ──\n");

    let api_key = std::env::var("GOOGLE_API_KEY")?;
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-3.1-flash-lite-preview")?);

    let agent = Arc::new(
        LlmAgentBuilder::new("language_services")
            .instruction(
                "You are a language services agent that provides translation, \
                 sentiment analysis, and summarization. When a user asks for \
                 language processing, use the appropriate tool. You can chain \
                 tools — e.g., translate then analyze sentiment of the translation.",
            )
            .model(model)
            .tool(Arc::new(Translate))
            .tool(Arc::new(AnalyzeSentiment))
            .tool(Arc::new(SummarizeText))
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

    // ── Part 3: Invoke the agent ──
    println!("  🤖 Agent: language_services");
    println!("     Tools: translate, analyze_sentiment, summarize_text\n");

    let message = Content::new("user").with_text(
        "Translate 'Rust makes systems programming accessible and safe' to Spanish, \
         then analyze the sentiment of the original text.",
    );

    print!("🤖 ");
    let mut stream = runner.run(uid, sid, message).await?;
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

    println!("\n\n=== Key Features ===");
    println!("• AWP Manifest — machine-readable capability declaration");
    println!("• Trust Levels — public, authenticated, verified (per-capability)");
    println!("• Rate Limiting — per-capability request quotas");
    println!("• Health Monitoring — /health endpoint for uptime checks");
    println!("• Event Streaming — SSE /events for real-time agent activity");
    println!("• Discovery — /.well-known/awp.json for agent-to-agent lookup");
    println!("• Consent — optional consent flow before capability invocation");
    Ok(())
}
