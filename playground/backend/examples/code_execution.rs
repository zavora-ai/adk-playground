//! Code Execution — agent with a sandboxed code tool
//!
//! Builds an agent equipped with a code execution tool backed by a mock
//! sandbox. The agent decides when to execute code, sends it to the tool,
//! and interprets the results — all via real LLM reasoning.

use adk_core::{SessionId, UserId};
use adk_rust::futures::StreamExt;
use adk_rust::prelude::*;
use adk_rust::session::{CreateRequest, SessionService};
use adk_tool::tool;
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Deserialize, JsonSchema)]
struct CodeExecArgs {
    /// The Rust code to execute (should contain a `fn run()` entry point)
    code: String,
    /// Optional JSON input to pass to the code
    input: Option<String>,
}

/// Execute Rust code in a sandboxed environment. Returns stdout output.
/// The code should define: fn run(input: serde_json::Value) -> serde_json::Value
#[tool]
async fn execute_code(args: CodeExecArgs) -> adk_tool::Result<serde_json::Value> {
    // Mock sandbox: simulate execution based on code content
    let code = &args.code;
    let (output, duration_ms) = if code.contains("fibonacci") || code.contains("fib") {
        (
            serde_json::json!({
                "result": [1, 1, 2, 3, 5, 8, 13, 21, 34, 55],
                "note": "First 10 Fibonacci numbers"
            }),
            12,
        )
    } else if code.contains("sort") {
        (
            serde_json::json!({
                "result": [1, 2, 3, 4, 5, 7, 8, 9],
                "note": "Sorted array"
            }),
            8,
        )
    } else if code.contains("factorial") {
        (
            serde_json::json!({
                "result": 120,
                "note": "5! = 120"
            }),
            5,
        )
    } else if code.contains("prime") {
        (
            serde_json::json!({
                "result": [2, 3, 5, 7, 11, 13, 17, 19, 23, 29],
                "note": "First 10 prime numbers"
            }),
            10,
        )
    } else {
        (
            serde_json::json!({
                "result": "executed successfully",
                "stdout": "Hello from sandbox!"
            }),
            3,
        )
    };

    Ok(serde_json::json!({
        "status": "success",
        "exit_code": 0,
        "duration_ms": duration_ms,
        "output": output,
        "sandbox": "playground-mock (production: use DockerBackend or WasmBackend)"
    }))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    println!("=== Code Execution Agent ===\n");

    let api_key = std::env::var("GOOGLE_API_KEY")?;
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-3.1-flash-lite-preview")?);

    let agent = Arc::new(
        LlmAgentBuilder::new("code_agent")
            .instruction(
                "You are a coding assistant with access to a Rust code execution sandbox.\n\
                 When the user asks you to compute something or run code:\n\
                 1. Use the execute_code tool with Rust source code\n\
                 2. The code should define: fn run(input: serde_json::Value) -> serde_json::Value\n\
                 3. Interpret the results and explain them clearly\n\
                 Be concise.",
            )
            .model(model)
            .tool(Arc::new(ExecuteCode))
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

    let query = "Write and run a Rust function that computes the first 10 Fibonacci numbers.";
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
