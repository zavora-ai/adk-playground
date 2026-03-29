use adk_core::{SessionId, UserId};
use adk_rust::futures::StreamExt;
use adk_rust::prelude::*;
use adk_rust::session::{CreateRequest, SessionService};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

// ── Anthropic Extended Thinking ──
// Claude's extended thinking mode lets the model reason internally in
// `<thinking>` blocks before responding. The thinking budget controls
// how many tokens Claude can spend on internal reasoning.
//
// Key concepts:
//   - `.with_thinking(budget)` — token budget for internal reasoning
//   - `Part::Thinking` — thinking content appears in the response stream
//   - Higher budgets allow deeper analysis of complex problems
//   - Thinking forces temperature=1.0 (Anthropic API requirement)
//
// This example gives Claude a complex systems design problem that
// requires weighing multiple tradeoffs — perfect for extended thinking.

#[derive(JsonSchema, Serialize, Deserialize)]
struct TradeoffArgs {
    /// The design option being evaluated
    option: String,
    /// Pros of this option
    pros: Vec<String>,
    /// Cons of this option
    cons: Vec<String>,
    /// Score from 1-10
    score: u8,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key =
        std::env::var("ANTHROPIC_API_KEY").expect("Set ANTHROPIC_API_KEY in your .env file");

    let tradeoff_tool = FunctionTool::new(
        "evaluate_tradeoff",
        "Record a design tradeoff evaluation with pros, cons, and a score",
        |_ctx, args| async move {
            let option = args.get("option").and_then(|v| v.as_str()).unwrap_or("?");
            let score = args.get("score").and_then(|v| v.as_u64()).unwrap_or(5);
            let pros: Vec<String> = args
                .get("pros")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let cons: Vec<String> = args
                .get("cons")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            Ok(serde_json::json!({
                "option": option,
                "pros": pros,
                "cons": cons,
                "score": score,
                "recorded": true,
            }))
        },
    )
    .with_parameters_schema::<TradeoffArgs>();

    // Extended thinking with 10K token budget for deep analysis
    let model = Arc::new(AnthropicClient::new(
        AnthropicConfig::new(&api_key, "claude-sonnet-4-5-20250929")
            .with_thinking(10240)
            .with_max_tokens(16384),
    )?);

    let agent = Arc::new(
        LlmAgentBuilder::new("systems_architect")
            .instruction(
                "You are a senior systems architect. Think deeply about design tradeoffs \
                 before responding. Use the evaluate_tradeoff tool to formally record each \
                 option you consider, scoring it 1-10. After evaluating all options, \
                 give your final recommendation with clear justification.",
            )
            .model(model)
            .tool(Arc::new(tradeoff_tool))
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

    println!("## 🔍 Anthropic Extended Thinking — Systems Design Analysis\n");
    println!("Claude will think internally (10K token budget) before responding.\n");

    let prompt = "We need to choose a data storage strategy for a social media platform \
             that handles 50M daily active users. The key requirements are:\n\
             - Sub-10ms read latency for user feeds\n\
             - Strong consistency for financial transactions (tips, subscriptions)\n\
             - Eventual consistency acceptable for likes/comments\n\
             - Must handle 500K writes/second at peak\n\
             - Budget: $50K/month infrastructure\n\n\
             Evaluate at least 3 different approaches (e.g., single DB, polyglot persistence, \
             CQRS+event sourcing) using the tradeoff tool, then recommend the best option.";
    println!(
        "<!--USER_PROMPT_START-->\n{}\n<!--USER_PROMPT_END-->",
        prompt
    );
    let message = Content::new("user").with_text(prompt);
    let mut stream = runner
        .run(UserId::new("user")?, SessionId::new("s1")?, message)
        .await?;

    let mut saw_thinking = false;
    while let Some(event) = stream.next().await {
        let event = event?;
        if let Some(content) = &event.llm_response.content {
            for part in &content.parts {
                match part {
                    Part::Thinking { thinking, .. } => {
                        saw_thinking = true;
                        println!("<!--THINKING_START-->\n{}\n<!--THINKING_END-->", thinking);
                    }
                    _ => {
                        if let Some(text) = part.text() {
                            print!("{}", text);
                        }
                    }
                }
            }
        }
    }
    println!();
    Ok(())
}
