use adk_core::{SessionId, UserId};
use adk_rust::futures::StreamExt;
use adk_rust::prelude::*;
use adk_rust::session::{CreateRequest, SessionService};
use adk_tool::tool;
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

// ── Azure AI Inference ──
// Access models deployed on Azure AI endpoints (Llama, Mistral, Cohere, etc.)
// via the Azure AI REST API. Each endpoint hosts a specific model deployment.
// Requires: AZURE_AI_ENDPOINT and AZURE_AI_API_KEY environment variables.

#[derive(Deserialize, JsonSchema)]
struct ClassifyArgs {
    /// Text to classify
    text: String,
}

/// Classify text into categories with confidence scores.
#[tool]
async fn classify_text(args: ClassifyArgs) -> adk_tool::Result<serde_json::Value> {
    let word_count = args.text.split_whitespace().count();
    let has_question = args.text.contains('?');
    let has_code =
        args.text.contains("fn ") || args.text.contains("def ") || args.text.contains("class ");

    let category = if has_code {
        "code"
    } else if has_question {
        "question"
    } else if word_count > 50 {
        "article"
    } else {
        "statement"
    };

    Ok(serde_json::json!({
        "text_preview": if args.text.len() > 80 { format!("{}...", &args.text[..80]) } else { args.text.clone() },
        "category": category,
        "word_count": word_count,
        "confidence": 0.87,
        "tags": match category {
            "code" => vec!["technical", "programming"],
            "question" => vec!["inquiry", "interactive"],
            "article" => vec!["long-form", "informational"],
            _ => vec!["general"],
        }
    }))
}

#[derive(Deserialize, JsonSchema)]
struct SummarizeArgs {
    /// Text to summarize
    text: String,
    /// Maximum number of sentences in summary
    max_sentences: Option<u32>,
}

/// Summarize text into key points.
#[tool]
async fn summarize_text(args: SummarizeArgs) -> adk_tool::Result<serde_json::Value> {
    let sentences: Vec<&str> = args.text.split(". ").filter(|s| !s.is_empty()).collect();
    let limit = args.max_sentences.unwrap_or(3) as usize;

    Ok(serde_json::json!({
        "original_sentences": sentences.len(),
        "summary_sentences": limit.min(sentences.len()),
        "key_points": sentences.iter().take(limit).collect::<Vec<_>>(),
        "compression_ratio": format!("{:.0}%", (1.0 - limit.min(sentences.len()) as f64 / sentences.len().max(1) as f64) * 100.0)
    }))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    let endpoint = std::env::var("AZURE_AI_ENDPOINT")
        .expect("Set AZURE_AI_ENDPOINT (e.g. https://my-endpoint.eastus.inference.ai.azure.com)");
    let api_key =
        std::env::var("AZURE_AI_API_KEY").expect("Set AZURE_AI_API_KEY in your .env file");
    let model_name =
        std::env::var("AZURE_AI_MODEL").unwrap_or_else(|_| "meta-llama-3.1-8b-instruct".into());

    let model = Arc::new(AzureAIClient::new(AzureAIConfig::new(
        &endpoint,
        &api_key,
        &model_name,
    ))?);

    let agent = Arc::new(
        LlmAgentBuilder::new("azure_text_analyst")
            .instruction(
                "You are a text analysis assistant deployed on Azure AI.\n\
                 Use classify_text to categorize input, and summarize_text for summaries.\n\
                 Always classify first, then summarize if the text is long enough.\n\
                 Present results clearly with the category, tags, and key points.",
            )
            .model(model)
            .tool(Arc::new(ClassifyText))
            .tool(Arc::new(SummarizeText))
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

    println!("☁️  Azure AI Inference — {} on Azure\n", model_name);

    let message = Content::new("user")
        .with_text(
            "Classify and summarize this: Rust is a systems programming language focused on safety, \
             speed, and concurrency. It achieves memory safety without garbage collection through its \
             ownership system. The borrow checker enforces strict rules at compile time. Rust is used \
             in web browsers, operating systems, game engines, and embedded devices. Companies like \
             Mozilla, Microsoft, Google, and Amazon use Rust in production."
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
