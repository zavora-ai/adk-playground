use adk_rust::graph::{StateGraph, NodeOutput, START, END};
use adk_rust::graph::ExecutionConfig;
use serde_json::json;
use std::collections::HashMap;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("=== Graph Workflow: Data Processing Pipeline ===\n");

    let graph = StateGraph::with_channels(&["text", "word_count", "sentiment", "summary"])
        // Node 1: Count words
        .add_node_fn("counter", |ctx| async move {
            let text = ctx.get("text").and_then(|v| v.as_str()).unwrap_or("");
            let count = text.split_whitespace().count();
            println!("[counter] {} words", count);
            Ok(NodeOutput::new().with_update("word_count", json!(count)))
        })
        // Node 2: Analyze sentiment (simple heuristic)
        .add_node_fn("analyzer", |ctx| async move {
            let text = ctx.get("text").and_then(|v| v.as_str()).unwrap_or("");
            let positive = ["great", "love", "excellent", "amazing", "good"];
            let negative = ["bad", "hate", "terrible", "awful", "poor"];
            let lower = text.to_lowercase();
            let pos: i32 = positive.iter().filter(|w| lower.contains(*w)).count() as i32;
            let neg: i32 = negative.iter().filter(|w| lower.contains(*w)).count() as i32;
            let sentiment = match pos - neg {
                s if s > 0 => "positive",
                s if s < 0 => "negative",
                _ => "neutral",
            };
            println!("[analyzer] sentiment = {}", sentiment);
            Ok(NodeOutput::new().with_update("sentiment", json!(sentiment)))
        })
        // Node 3: Generate summary
        .add_node_fn("summarizer", |ctx| async move {
            let count = ctx.get("word_count").and_then(|v| v.as_u64()).unwrap_or(0);
            let sentiment = ctx.get("sentiment").and_then(|v| v.as_str()).unwrap_or("unknown");
            let summary = format!(
                "Text has {} words with {} sentiment.",
                count, sentiment
            );
            println!("[summarizer] {}", summary);
            Ok(NodeOutput::new().with_update("summary", json!(summary)))
        })
        // Edges: START → counter → analyzer → summarizer → END
        .add_edge(START, "counter")
        .add_edge("counter", "analyzer")
        .add_edge("analyzer", "summarizer")
        .add_edge("summarizer", END)
        .compile()?;

    // Run the graph
    let mut input = HashMap::new();
    input.insert(
        "text".to_string(),
        json!("Rust is an amazing language. The borrow checker is great \
               and the community is excellent. Performance is good too."),
    );

    let result = graph.invoke(input, ExecutionConfig::new("run-1")).await?;

    println!("\n=== Results ===");
    println!("Words: {}", result.get("word_count").unwrap_or(&json!(null)));
    println!("Sentiment: {}", result.get("sentiment").unwrap_or(&json!(null)));
    println!("Summary: {}", result.get("summary").unwrap_or(&json!(null)));
    Ok(())
}
