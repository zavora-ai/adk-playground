use adk_core::{SessionId, UserId};
use adk_rust::futures::StreamExt;
use adk_rust::prelude::*;
use adk_rust::session::{CreateRequest, SessionService};
use adk_tool::tool;
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

// ── Mistral Medium via OpenAI-Compatible API ──
// Showcases Mistral's multilingual strength and function calling.
// Uses `OpenAICompatibleConfig::mistral()` preset — one line to connect.
// Mistral models excel at European languages and structured tool use.

#[derive(Deserialize, JsonSchema)]
struct TranslateArgs {
    /// Text to translate
    text: String,
    /// Target language code (e.g., "fr", "de", "es", "ja")
    target_language: String,
}

/// Translate text to the target language.
#[tool]
async fn translate(args: TranslateArgs) -> adk_tool::Result<serde_json::Value> {
    // Simulated translation service
    let translated = match args.target_language.as_str() {
        "fr" => format!(
            "[FR] {}",
            args.text
                .replace("Hello", "Bonjour")
                .replace("world", "monde")
        ),
        "de" => format!(
            "[DE] {}",
            args.text.replace("Hello", "Hallo").replace("world", "Welt")
        ),
        "es" => format!(
            "[ES] {}",
            args.text.replace("Hello", "Hola").replace("world", "mundo")
        ),
        "ja" => format!(
            "[JA] {}",
            args.text
                .replace("Hello", "こんにちは")
                .replace("world", "世界")
        ),
        _ => format!("[{}] {}", args.target_language.to_uppercase(), args.text),
    };
    Ok(serde_json::json!({
        "original": args.text,
        "translated": translated,
        "target_language": args.target_language,
    }))
}

#[derive(Deserialize, JsonSchema)]
struct SentimentArgs {
    /// Text to analyze
    text: String,
}

/// Analyze the sentiment of text.
#[tool]
async fn analyze_sentiment(args: SentimentArgs) -> adk_tool::Result<serde_json::Value> {
    let score = if args.text.contains("love")
        || args.text.contains("great")
        || args.text.contains("excellent")
    {
        0.9
    } else if args.text.contains("hate") || args.text.contains("terrible") {
        -0.8
    } else {
        0.1
    };
    Ok(serde_json::json!({
        "text": args.text,
        "sentiment": if score > 0.5 { "positive" } else if score < -0.3 { "negative" } else { "neutral" },
        "score": score,
    }))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key = std::env::var("MISTRAL_API_KEY").expect("Set MISTRAL_API_KEY in your .env file");

    // Mistral Medium: strong multilingual + tool calling
    let model = Arc::new(OpenAICompatible::new(OpenAICompatibleConfig::mistral(
        api_key,
        "mistral-medium-latest",
    ))?);

    let agent = Arc::new(
        LlmAgentBuilder::new("multilingual_assistant")
            .instruction(
                "You are a multilingual assistant. You can translate text and analyze sentiment. \
                 When asked to process text in multiple languages, use the appropriate tools. \
                 Always respond with a brief summary of what you did.",
            )
            .model(model)
            .tool(Arc::new(Translate))
            .tool(Arc::new(AnalyzeSentiment))
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

    println!("🌍 Mistral Medium — Multilingual Tools\n");

    let message = Content::new("user").with_text(
        "Translate 'Hello world, Rust is great!' to French and Spanish, \
             then analyze the sentiment of the original text.",
    );
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
