use adk_rust::prelude::*;
use adk_tool::tool;
use adk_rust::tool::AgentTool;
use adk_rust::session::{SessionService, CreateRequest};
use adk_rust::futures::StreamExt;
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Deserialize, JsonSchema)]
struct CalcArgs {
    /// Arithmetic operation: add, subtract, multiply, divide
    operation: String,
    /// First number
    a: f64,
    /// Second number
    b: f64,
}

/// Perform arithmetic on two numbers.
#[tool]
async fn calculator(args: CalcArgs) -> adk_tool::Result<serde_json::Value> {
    let result = match args.operation.as_str() {
        "add" => args.a + args.b,
        "subtract" => args.a - args.b,
        "multiply" => args.a * args.b,
        "divide" if args.b != 0.0 => args.a / args.b,
        "divide" => return Ok(serde_json::json!({"error": "division by zero"})),
        _ => return Ok(serde_json::json!({"error": "unknown operation"})),
    };
    Ok(serde_json::json!({"result": result}))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key = std::env::var("GOOGLE_API_KEY")?;
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-3.1-flash-lite-preview")?);

    // Specialist: math agent with a calculator tool
    let math_agent = LlmAgentBuilder::new("math_expert")
        .description("Solves math problems using a calculator tool")
        .instruction("You are a math expert. Use the calculator for arithmetic. Show your work.")
        .model(model.clone())
        .tool(Arc::new(Calculator))
        .build()?;

    // Specialist: trivia agent (LLM knowledge only)
    let trivia_agent = LlmAgentBuilder::new("trivia_expert")
        .description("Answers trivia and general knowledge questions")
        .instruction("You are a trivia expert. Answer accurately and concisely.")
        .model(model.clone())
        .build()?;

    // Wrap specialists as tools for the coordinator
    let math_tool = AgentTool::new(Arc::new(math_agent));
    let trivia_tool = AgentTool::new(Arc::new(trivia_agent));

    let coordinator = Arc::new(
        LlmAgentBuilder::new("coordinator")
            .instruction(
                "Route questions to the right specialist:\n\
                 - Math/calculations → math_expert\n\
                 - Trivia/facts → trivia_expert\n\
                 Summarize the specialist's response for the user."
            )
            .model(model)
            .tool(Arc::new(math_tool))
            .tool(Arc::new(trivia_tool))
            .build()?
    );

    let sessions = Arc::new(InMemorySessionService::new());
    sessions.create(CreateRequest {
        app_name: "playground".into(),
        user_id: "user".into(),
        session_id: Some("s1".into()),
        state: HashMap::new(),
    }).await?;

    let runner = Runner::new(RunnerConfig {
        app_name: "playground".into(),
        agent: coordinator,
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

    let message = Content::new("user")
        .with_text("What is 15% of 250, and who invented the percentage symbol?");
    let mut stream = runner.run("user".into(), "s1".into(), message).await?;

    while let Some(event) = stream.next().await {
        let event = event?;
        if let Some(content) = &event.llm_response.content {
            for part in &content.parts {
                if let Some(text) = part.text() { print!("{}", text); }
            }
        }
    }
    println!();
    Ok(())
}
