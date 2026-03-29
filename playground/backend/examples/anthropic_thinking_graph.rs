use adk_rust::graph::{AgentNode, ExecutionConfig};
use adk_rust::graph::{NodeOutput, StateGraph, END, START};
use adk_rust::prelude::*;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

// ── Anthropic Extended Thinking — Graph Pipeline ──
// Combines Claude's extended thinking with a StateGraph pipeline.
// The "thinker" agent uses `.with_thinking(10240)` for deep analysis,
// then a "summarizer" agent condenses the analysis into actionable points.
// Shows Part::Thinking blocks in the output alongside the graph flow.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key =
        std::env::var("ANTHROPIC_API_KEY").expect("Set ANTHROPIC_API_KEY in your .env file");

    // Thinker: extended thinking enabled for deep reasoning
    let thinker_model = Arc::new(AnthropicClient::new(
        AnthropicConfig::new(&api_key, "claude-sonnet-4-20250514")
            .with_thinking(10240)
            .with_max_tokens(8192),
    )?);

    // Summarizer: standard mode, concise output
    let summarizer_model = Arc::new(AnthropicClient::new(
        AnthropicConfig::new(&api_key, "claude-sonnet-4-20250514")
            .with_max_tokens(1024),
    )?);

    println!("=== Anthropic Extended Thinking — Graph Pipeline ===");
    println!("Thinker (10K thinking budget) → Summarizer (concise output)\n");

    let thinker = Arc::new(
        LlmAgentBuilder::new("thinker")
            .description("Deep analysis agent with extended thinking")
            .model(thinker_model)
            .instruction(
                "You are a systems design expert. Think deeply about the problem \
                 before responding. Analyze trade-offs, edge cases, and failure modes. \
                 Provide a thorough technical analysis.",
            )
            .build()?,
    );

    let summarizer = Arc::new(
        LlmAgentBuilder::new("summarizer")
            .description("Concise summarizer")
            .model(summarizer_model)
            .instruction(
                "You are a technical editor. Take the detailed analysis provided and \
                 distill it into 3-5 bullet points. Each bullet should be one sentence. \
                 Focus on actionable recommendations.",
            )
            .build()?,
    );

    // Graph: START → thinker → summarizer → END
    let thinker_node = AgentNode::new(thinker)
        .with_input_mapper(|state| {
            let question = state.get("question").and_then(|v| v.as_str()).unwrap_or("");
            Content::new("user").with_text(question)
        })
        .with_output_mapper(|events| {
            let mut updates = HashMap::new();
            let mut analysis = String::new();
            let mut thinking_tokens = 0u32;
            for event in events {
                if let Some(content) = event.content() {
                    for part in &content.parts {
                        match part {
                            adk_core::Part::Thinking { thinking, .. } => {
                                thinking_tokens += 1;
                                let preview = &thinking[..thinking.len().min(150)];
                                println!("  💭 Thinking: {}...", preview);
                            }
                            adk_core::Part::Text { text } => {
                                analysis.push_str(text);
                            }
                            _ => {}
                        }
                    }
                }
            }
            println!("\n  🧠 Thinker produced {} thinking blocks, {} chars of analysis",
                thinking_tokens, analysis.len());
            updates.insert("analysis".to_string(), json!(analysis));
            updates
        });

    let summarizer_node = AgentNode::new(summarizer)
        .with_input_mapper(|state| {
            let analysis = state.get("analysis").and_then(|v| v.as_str()).unwrap_or("");
            Content::new("user").with_text(format!(
                "Summarize this technical analysis into 3-5 actionable bullet points:\n\n{}",
                analysis
            ))
        })
        .with_output_mapper(|events| {
            let mut updates = HashMap::new();
            let mut summary = String::new();
            for event in events {
                if let Some(content) = event.content() {
                    for part in &content.parts {
                        if let Some(text) = part.text() {
                            summary.push_str(text);
                        }
                    }
                }
            }
            updates.insert("summary".to_string(), json!(summary));
            updates
        });

    let graph = StateGraph::with_channels(&["question", "analysis", "summary"])
        .add_node(thinker_node)
        .add_node(summarizer_node)
        .add_edge(START, "thinker")
        .add_edge("thinker", "summarizer")
        .add_edge("summarizer", END)
        .compile()?;

    let mut input = HashMap::new();
    input.insert(
        "question".to_string(),
        json!("Should a startup building a real-time collaborative editor use CRDTs or \
               Operational Transform? Consider latency, complexity, offline support, \
               and team size (3 engineers)."),
    );

    println!("❓ Question: CRDTs vs Operational Transform for a collaborative editor\n");
    println!("── Phase 1: Deep Analysis (Extended Thinking) ──\n");

    let result = graph.invoke(input, ExecutionConfig::new("think-1")).await?;

    println!("\n── Phase 2: Actionable Summary ──\n");
    if let Some(summary) = result.get("summary").and_then(|v| v.as_str()) {
        println!("{}", summary);
    }

    println!("\n=== Extended Thinking in Graphs ===");
    println!("• with_thinking(10240) gives Claude a 10K token reasoning budget");
    println!("• Part::Thinking blocks show internal reasoning (not sent to user)");
    println!("• Graph pipeline: deep thinker → concise summarizer");
    println!("• Ideal for complex analysis that needs both depth and clarity");
    Ok(())
}
