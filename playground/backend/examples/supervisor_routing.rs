use adk_rust::prelude::*;
use adk_rust::graph::{StateGraph, NodeOutput, START, END};
use adk_rust::graph::{AgentNode, ExecutionConfig};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key = std::env::var("GOOGLE_API_KEY")?;
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-3.1-flash-lite-preview")?);

    println!("=== Supervisor Routing: Task Delegation ===\n");

    // Supervisor classifies and routes tasks
    let supervisor = Arc::new(
        LlmAgentBuilder::new("supervisor")
            .model(model.clone())
            .instruction(
                "You are a task supervisor. Route to the appropriate specialist:\n\
                 - 'researcher' for research/analysis tasks\n\
                 - 'writer' for writing/content tasks\n\
                 - 'coder' for programming/technical tasks\n\
                 - 'done' if the task is complete\n\n\
                 Reply with ONLY the specialist name."
            )
            .build()?
    );

    // Specialist agents
    let researcher = Arc::new(
        LlmAgentBuilder::new("researcher")
            .model(model.clone())
            .instruction("You are a research specialist. Provide detailed findings. Keep under 3 sentences.")
            .build()?
    );
    let writer = Arc::new(
        LlmAgentBuilder::new("writer")
            .model(model.clone())
            .instruction("You are a writing specialist. Create clear, engaging content. Keep under 3 sentences.")
            .build()?
    );
    let coder = Arc::new(
        LlmAgentBuilder::new("coder")
            .model(model)
            .instruction("You are a coding specialist. Provide technical solutions. Keep under 3 sentences.")
            .build()?
    );

    // Build nodes with input/output mappers
    let supervisor_node = AgentNode::new(supervisor)
        .with_input_mapper(|state| {
            let task = state.get("task").and_then(|v| v.as_str()).unwrap_or("");
            let history = state.get("work_done").and_then(|v| v.as_str()).unwrap_or("");
            let prompt = if history.is_empty() {
                format!("Task: {}", task)
            } else {
                format!("Task: {}\nWork done: {}\nIs this complete? Reply: researcher, writer, coder, or done.", task, history)
            };
            Content::new("user").with_text(&prompt)
        })
        .with_output_mapper(|events| {
            let mut updates = HashMap::new();
            for event in events {
                if let Some(content) = event.content() {
                    let text: String = content.parts.iter()
                        .filter_map(|p| p.text()).collect::<Vec<_>>().join("")
                        .to_lowercase().trim().to_string();
                    let next = if text.contains("researcher") { "researcher" }
                        else if text.contains("writer") { "writer" }
                        else if text.contains("coder") { "coder" }
                        else { "done" };
                    println!("📋 Supervisor → {}", next);
                    updates.insert("next_agent".to_string(), json!(next));
                }
            }
            updates
        });

    let make_specialist_node = |agent: Arc<dyn Agent + Send + Sync>, emoji: &'static str| {
        AgentNode::new(agent.clone())
            .with_input_mapper(|state| {
                let task = state.get("task").and_then(|v| v.as_str()).unwrap_or("");
                Content::new("user").with_text(task)
            })
            .with_output_mapper(move |events| {
                let mut updates = HashMap::new();
                for event in events {
                    if let Some(content) = event.content() {
                        let text: String = content.parts.iter()
                            .filter_map(|p| p.text()).collect::<Vec<_>>().join("");
                        println!("{} Result: {}", emoji, &text[..text.len().min(100)]);
                        updates.insert("work_done".to_string(), json!(text));
                    }
                }
                updates
            })
    };

    let researcher_node = make_specialist_node(researcher, "🔍");
    let writer_node = make_specialist_node(writer, "✍️");
    let coder_node = make_specialist_node(coder, "💻");

    let graph = StateGraph::with_channels(&["task", "next_agent", "work_done", "iteration"])
        .add_node(supervisor_node)
        .add_node(researcher_node)
        .add_node(writer_node)
        .add_node(coder_node)
        .add_node_fn("counter", |ctx| async move {
            let i = ctx.get("iteration").and_then(|v| v.as_i64()).unwrap_or(0);
            Ok(NodeOutput::new().with_update("iteration", json!(i + 1)))
        })
        .add_edge(START, "counter")
        .add_edge("counter", "supervisor")
        .add_conditional_edges(
            "supervisor",
            |state| {
                let next = state.get("next_agent").and_then(|v| v.as_str()).unwrap_or("done");
                let iteration = state.get("iteration").and_then(|v| v.as_i64()).unwrap_or(0);
                if iteration >= 3 { return END.to_string(); }
                next.to_string()
            },
            [("researcher", "researcher"), ("writer", "writer"), ("coder", "coder"), ("done", END), (END, END)],
        )
        .add_edge("researcher", "counter")
        .add_edge("writer", "counter")
        .add_edge("coder", "counter")
        .compile()?
        .with_recursion_limit(10);

    let mut input = HashMap::new();
    input.insert("task".to_string(), json!("Research the benefits of WebAssembly and write a brief summary"));

    let result = graph.invoke(input, ExecutionConfig::new("task-1")).await?;
    println!("\n✅ Final: {}", result.get("work_done").and_then(|v| v.as_str()).unwrap_or(""));
    Ok(())
}
