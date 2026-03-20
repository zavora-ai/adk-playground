use adk_rust::prelude::*;
use adk_tool::tool;
use adk_rust::graph::{StateGraph, NodeOutput, START, END};
use adk_rust::graph::{AgentNode, ExecutionConfig};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Deserialize, JsonSchema)]
struct LocationQuery {
    /// The city or location to check weather for
    location: String,
}

/// Get the current weather for a location.
#[tool]
async fn get_weather(args: LocationQuery) -> adk_tool::Result<serde_json::Value> {
    println!("  🌤️ Getting weather for: {}", args.location);
    Ok(json!({ "location": args.location, "temperature": "72°F", "condition": "Sunny" }))
}

#[derive(Deserialize, JsonSchema)]
struct MathExpr {
    /// The mathematical expression to evaluate
    expression: String,
}

/// Perform a mathematical calculation.
#[tool]
async fn calculator(args: MathExpr) -> adk_tool::Result<serde_json::Value> {
    println!("  🧮 Calculating: {}", args.expression);
    let result = match args.expression.as_str() {
        "15 + 25" => "40",
        "100 / 4" => "25",
        _ => "Unable to evaluate",
    };
    Ok(json!({ "result": result, "expression": args.expression }))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key = std::env::var("GOOGLE_API_KEY")?;
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-3.1-flash-lite-preview")?);

    println!("=== ReAct Pattern: Iterative Reasoning with Tools ===\n");

    let reasoner = Arc::new(
        LlmAgentBuilder::new("reasoner")
            .description("Reasoning agent with tools")
            .model(model)
            .instruction(
                "You are a helpful assistant with tools. Use them when needed. \
                 When you have enough information, provide a final answer."
            )
            .tool(Arc::new(GetWeather))
            .tool(Arc::new(Calculator))
            .build()?
    );

    // ReAct graph: reason → check if tools used → loop or finish
    let reasoner_node = AgentNode::new(reasoner)
        .with_input_mapper(|state| {
            let question = state.get("question").and_then(|v| v.as_str()).unwrap_or("");
            Content::new("user").with_text(question)
        })
        .with_output_mapper(|events| {
            let mut updates = HashMap::new();
            let mut has_tool_calls = false;
            let mut response = String::new();
            for event in events {
                if let Some(content) = event.content() {
                    for part in &content.parts {
                        match part {
                            Part::FunctionCall { name, .. } => {
                                println!("🔧 Tool called: {}", name);
                                has_tool_calls = true;
                            }
                            Part::Text { text } => {
                                response.push_str(text);
                            }
                            _ => {}
                        }
                    }
                }
            }
            updates.insert("has_tool_calls".to_string(), json!(has_tool_calls));
            updates.insert("response".to_string(), json!(response));
            updates
        });

    let graph = StateGraph::with_channels(&["question", "has_tool_calls", "response", "iteration"])
        .add_node(reasoner_node)
        .add_node_fn("counter", |ctx| async move {
            let i = ctx.get("iteration").and_then(|v| v.as_i64()).unwrap_or(0);
            println!("📊 Iteration: {}", i + 1);
            Ok(NodeOutput::new().with_update("iteration", json!(i + 1)))
        })
        .add_edge(START, "counter")
        .add_edge("counter", "reasoner")
        .add_conditional_edges(
            "reasoner",
            |state| {
                let has_tools = state.get("has_tool_calls").and_then(|v| v.as_bool()).unwrap_or(false);
                let iteration = state.get("iteration").and_then(|v| v.as_i64()).unwrap_or(0);
                if iteration >= 3 { return END.to_string(); }
                if has_tools { "counter".to_string() } else { END.to_string() }
            },
            [("counter", "counter"), (END, END)],
        )
        .compile()?
        .with_recursion_limit(5);

    let mut input = HashMap::new();
    input.insert("question".to_string(), json!("What's the weather in Paris and what's 15 + 25?"));

    let result = graph.invoke(input, ExecutionConfig::new("react-1")).await?;

    println!("\n🎯 Final Answer: {}", result.get("response").and_then(|v| v.as_str()).unwrap_or(""));
    println!("Iterations: {}", result.get("iteration").and_then(|v| v.as_i64()).unwrap_or(0));
    Ok(())
}
