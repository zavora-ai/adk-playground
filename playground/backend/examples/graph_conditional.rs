use adk_rust::graph::{StateGraph, NodeOutput, START, END};
use adk_rust::graph::ExecutionConfig;
use serde_json::json;
use std::collections::HashMap;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("=== Graph: Conditional Routing ===\n");

    let graph = StateGraph::with_channels(&["message", "priority", "response"])
        // Classify priority
        .add_node_fn("classifier", |ctx| async move {
            let msg = ctx.get("message").and_then(|v| v.as_str()).unwrap_or("");
            let lower = msg.to_lowercase();
            let priority = if lower.contains("urgent") || lower.contains("emergency") {
                "high"
            } else if lower.contains("help") || lower.contains("issue") {
                "medium"
            } else {
                "low"
            };
            println!("[classifier] priority = {}", priority);
            Ok(NodeOutput::new().with_update("priority", json!(priority)))
        })
        // High priority handler
        .add_node_fn("high_priority", |ctx| async move {
            let msg = ctx.get("message").and_then(|v| v.as_str()).unwrap_or("");
            let response = format!("🚨 URGENT: Escalating immediately. Message: {}", msg);
            Ok(NodeOutput::new().with_update("response", json!(response)))
        })
        // Medium priority handler
        .add_node_fn("medium_priority", |ctx| async move {
            let msg = ctx.get("message").and_then(|v| v.as_str()).unwrap_or("");
            let response = format!("📋 Ticket created for: {}", msg);
            Ok(NodeOutput::new().with_update("response", json!(response)))
        })
        // Low priority handler
        .add_node_fn("low_priority", |ctx| async move {
            let msg = ctx.get("message").and_then(|v| v.as_str()).unwrap_or("");
            let response = format!("ℹ️ Added to queue: {}", msg);
            Ok(NodeOutput::new().with_update("response", json!(response)))
        })
        .add_edge(START, "classifier")
        // Route based on priority field
        .add_conditional_edges(
            "classifier",
            |state| {
                state.get("priority")
                    .and_then(|v| v.as_str())
                    .unwrap_or("low")
                    .to_string()
            },
            [
                ("high", "high_priority"),
                ("medium", "medium_priority"),
                ("low", "low_priority"),
            ],
        )
        .add_edge("high_priority", END)
        .add_edge("medium_priority", END)
        .add_edge("low_priority", END)
        .compile()?;

    // Test with different messages
    let messages = [
        "URGENT: Server is down and customers can't access the site!",
        "I need help resetting my password",
        "Just checking if you have a dark mode option",
    ];

    for msg in messages {
        println!("Input: \"{}\"\n", msg);
        let mut input = HashMap::new();
        input.insert("message".to_string(), json!(msg));
        let result = graph.invoke(input, ExecutionConfig::new("test")).await?;
        println!(
            "→ {}\n",
            result.get("response").and_then(|v| v.as_str()).unwrap_or("No response")
        );
    }
    Ok(())
}
