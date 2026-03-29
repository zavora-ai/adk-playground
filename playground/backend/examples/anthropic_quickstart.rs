use adk_core::{SessionId, UserId};
use adk_rust::futures::StreamExt;
use adk_rust::prelude::*;
use adk_rust::session::{CreateRequest, SessionService};
use adk_tool::tool;
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

// ── Anthropic Claude Sonnet 4.5 with Extended Thinking ──
// Showcases Claude's unique extended thinking: the model reasons internally
// in `thinking` blocks before responding. Great for complex analysis tasks.
// `.with_thinking(10240)` sets a 10K token budget for internal reasoning.

#[derive(Deserialize, JsonSchema)]
struct AnalyzeCodeArgs {
    /// Source code to analyze
    code: String,
    /// Programming language
    language: String,
}

/// Analyze code for bugs, performance issues, and security concerns.
#[tool]
async fn analyze_code(args: AnalyzeCodeArgs) -> adk_tool::Result<serde_json::Value> {
    Ok(serde_json::json!({
        "language": args.language,
        "lines": args.code.lines().count(),
        "issues": [
            {"severity": "high", "type": "security", "description": "SQL string concatenation — vulnerable to injection", "line": 3},
            {"severity": "medium", "type": "performance", "description": "N+1 query pattern in loop", "line": 7},
            {"severity": "low", "type": "style", "description": "Unused variable `temp`", "line": 12}
        ],
        "complexity_score": 7.2
    }))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key =
        std::env::var("ANTHROPIC_API_KEY").expect("Set ANTHROPIC_API_KEY in your .env file");

    // Claude Sonnet 4.5 with extended thinking (10K token budget)
    let model = Arc::new(AnthropicClient::new(
        AnthropicConfig::new(api_key, "claude-sonnet-4-5-20250929")
            .with_thinking(10240)
            .with_max_tokens(16384),
    )?);

    let agent = Arc::new(
        LlmAgentBuilder::new("code_reviewer")
            .instruction(
                "You are a senior code reviewer. Use the analyze_code tool to scan code, \
                 then provide a detailed review with prioritized fixes. Think carefully \
                 about the security implications before responding.",
            )
            .model(model)
            .tool(Arc::new(AnalyzeCode))
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

    println!("🔍 Claude Sonnet 4.5 — Extended Thinking + Code Review\n");

    let code_snippet = r#"fn get_user(db: &Database, name: &str) -> User {
    let query = format!("SELECT * FROM users WHERE name = '{}'", name);
    let result = db.execute(&query).unwrap();
    let mut users = Vec::new();
    for row in result.rows() {
        let user = User::from_row(row);
        let orders = db.execute(&format!("SELECT * FROM orders WHERE user_id = {}", user.id)).unwrap();
        user.orders = orders.into();
        users.push(user);
    }
    let temp = users.len();
    users.into_iter().next().unwrap()
}"#;

    let message = Content::new("user").with_text(format!(
        "Review this Rust function for issues:\n```rust\n{}\n```",
        code_snippet
    ));
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
