use adk_rust::graph::{AgentNode, ExecutionConfig};
use adk_rust::graph::{NodeOutput, StateGraph, END, START};
use adk_rust::prelude::*;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

// ── Graph Pipeline: LLM Agents in a Sequential Graph ──
// Three LLM agents connected in a graph: Analyst → Writer → Editor.
// Deterministic nodes handle data prep between agents.
// Shows how StateGraph orchestrates multi-agent pipelines with
// typed state channels flowing data between nodes.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key = std::env::var("GOOGLE_API_KEY")?;
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-2.5-flash")?);

    println!("=== Graph Pipeline: Multi-Agent Data Flow ===\n");

    // ── LLM Agents ──
    let analyst = Arc::new(
        LlmAgentBuilder::new("analyst")
            .model(model.clone())
            .instruction(
                "You are a data analyst. Given raw text, identify the key themes, \
                 sentiment, and 2-3 main points. Be concise — bullet points, under 80 words.",
            )
            .build()?,
    );

    let writer = Arc::new(
        LlmAgentBuilder::new("writer")
            .model(model.clone())
            .instruction(
                "You are a copywriter. Given an analysis brief, write a short engaging \
                 summary paragraph (40-60 words) suitable for a newsletter. \
                 Make it punchy and professional.",
            )
            .build()?,
    );

    let editor = Arc::new(
        LlmAgentBuilder::new("editor")
            .model(model)
            .instruction(
                "You are an editor. Given a draft, improve clarity and add a one-line \
                 headline. Output format:\nHEADLINE: ...\nBODY: ...\nKeep it under 80 words total.",
            )
            .build()?,
    );

    // ── Graph: prep → analyst → format → writer → editor → END ──
    let analyst_node = AgentNode::new(analyst)
        .with_input_mapper(|state| {
            let text = state.get("raw_text").and_then(|v| v.as_str()).unwrap_or("");
            Content::new("user").with_text(format!("Analyze this text:\n\n{}", text))
        })
        .with_output_mapper(|events| {
            let mut updates = HashMap::new();
            let mut analysis = String::new();
            for event in events {
                if let Some(c) = event.content() {
                    for p in &c.parts { if let Some(t) = p.text() { analysis.push_str(t); } }
                }
            }
            println!("🔍 Analyst: {} chars", analysis.len());
            updates.insert("analysis".to_string(), json!(analysis));
            updates
        });

    let writer_node = AgentNode::new(writer)
        .with_input_mapper(|state| {
            let analysis = state.get("analysis").and_then(|v| v.as_str()).unwrap_or("");
            Content::new("user").with_text(format!(
                "Write a newsletter summary from this analysis:\n\n{}", analysis
            ))
        })
        .with_output_mapper(|events| {
            let mut updates = HashMap::new();
            let mut draft = String::new();
            for event in events {
                if let Some(c) = event.content() {
                    for p in &c.parts { if let Some(t) = p.text() { draft.push_str(t); } }
                }
            }
            println!("✍️  Writer: {} chars", draft.len());
            updates.insert("draft".to_string(), json!(draft));
            updates
        });

    let editor_node = AgentNode::new(editor)
        .with_input_mapper(|state| {
            let draft = state.get("draft").and_then(|v| v.as_str()).unwrap_or("");
            Content::new("user").with_text(format!("Edit and add a headline:\n\n{}", draft))
        })
        .with_output_mapper(|events| {
            let mut updates = HashMap::new();
            let mut final_text = String::new();
            for event in events {
                if let Some(c) = event.content() {
                    for p in &c.parts { if let Some(t) = p.text() { final_text.push_str(t); } }
                }
            }
            updates.insert("final_output".to_string(), json!(final_text));
            updates
        });

    let graph = StateGraph::with_channels(&[
        "raw_text", "word_count", "analysis", "draft", "final_output",
    ])
    // Deterministic prep: count words
    .add_node_fn("prep", |ctx| async move {
        let text = ctx.get("raw_text").and_then(|v| v.as_str()).unwrap_or("");
        let count = text.split_whitespace().count();
        println!("📦 Prep: {} words ingested", count);
        Ok(NodeOutput::new().with_update("word_count", json!(count)))
    })
    // LLM agents
    .add_node(analyst_node)
    .add_node(writer_node)
    .add_node(editor_node)
    // Flow: START → prep → analyst → writer → editor → END
    .add_edge(START, "prep")
    .add_edge("prep", "analyst")
    .add_edge("analyst", "writer")
    .add_edge("writer", "editor")
    .add_edge("editor", END)
    .compile()?;

    let mut input = HashMap::new();
    input.insert("raw_text".to_string(), json!(
        "Rust 1.80 introduces lazy type aliases, stabilizes the `impl Trait` in \
         associated types, and brings pattern types to nightly. The community is \
         excited about the performance improvements in the compiler, with build \
         times dropping 15% on average. The async ecosystem continues to mature \
         with Tokio 2.0 on the horizon."
    ));

    let result = graph.invoke(input, ExecutionConfig::new("pipeline-1")).await?;

    println!("\n=== Final Output ===\n");
    println!("{}", result.get("final_output").and_then(|v| v.as_str()).unwrap_or(""));
    println!("\n📊 Pipeline: {} words → analyst → writer → editor",
        result.get("word_count").and_then(|v| v.as_u64()).unwrap_or(0));
    Ok(())
}
