use adk_rust::graph::{AgentNode, ExecutionConfig};
use adk_rust::graph::{NodeOutput, StateGraph, END, START};
use adk_rust::prelude::*;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

// ── Action + Agent: Content Pipeline ──
//
// A realistic content creation pipeline mixing action nodes with agents:
//   1. SET action — loads topic config and constraints
//   2. LLM Agent (researcher) — researches the topic
//   3. TRANSFORM action — formats research into a brief
//   4. LLM Agent (writer) — writes the article from the brief
//   5. TRANSFORM action — adds metadata (word count, reading time)
//   6. SWITCH — routes based on quality (word count threshold)
//   7. LLM Agent (editor) — polishes if needed, or skip to publish
//
// Action nodes handle data plumbing; agents handle creative work.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key = std::env::var("GOOGLE_API_KEY")?;
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-3.1-flash-lite-preview")?);

    println!("=== Action + Agent: Content Pipeline ===");
    println!("    SET → Research → TRANSFORM → Write → TRANSFORM → SWITCH → Edit/Publish\n");

    // Research agent
    let researcher = Arc::new(
        LlmAgentBuilder::new("researcher")
            .model(model.clone())
            .instruction(
                "You are a research assistant. Given a topic, provide 3-4 key facts \
                 or talking points. Be specific and factual. Use bullet points.",
            )
            .build()?,
    );

    // Writer agent
    let writer = Arc::new(
        LlmAgentBuilder::new("writer")
            .model(model.clone())
            .instruction(
                "You are a blog writer. Given research notes, write a short blog post \
                 (4-6 sentences). Make it engaging and informative. Include a catchy opening.",
            )
            .build()?,
    );

    // Editor agent
    let editor = Arc::new(
        LlmAgentBuilder::new("editor")
            .model(model)
            .instruction(
                "You are an editor. Polish the draft: tighten prose, fix flow, \
                 add a strong closing sentence. Keep the same length. Return the improved version only.",
            )
            .build()?,
    );

    // Agent nodes with mappers
    let researcher_node = AgentNode::new(researcher)
        .with_input_mapper(|state| {
            let topic = state.get("topic").and_then(|v| v.as_str()).unwrap_or("technology");
            let audience = state.get("audience").and_then(|v| v.as_str()).unwrap_or("developers");
            Content::new("user").with_text(&format!(
                "Research this topic for a {} audience: {}", audience, topic
            ))
        })
        .with_output_mapper(|events| {
            let mut updates = HashMap::new();
            let mut research = String::new();
            for event in events {
                if let Some(content) = event.content() {
                    for part in &content.parts {
                        if let Some(text) = part.text() { research.push_str(text); }
                    }
                }
            }
            println!("  🔍 Research: {} chars", research.len());
            updates.insert("research".to_string(), json!(research));
            updates
        });

    let writer_node = AgentNode::new(writer)
        .with_input_mapper(|state| {
            let brief = state.get("brief").and_then(|v| v.as_str()).unwrap_or("");
            Content::new("user").with_text(&format!("Write a blog post from this brief:\n{}", brief))
        })
        .with_output_mapper(|events| {
            let mut updates = HashMap::new();
            let mut draft = String::new();
            for event in events {
                if let Some(content) = event.content() {
                    for part in &content.parts {
                        if let Some(text) = part.text() { draft.push_str(text); }
                    }
                }
            }
            println!("  ✍️  Draft: {} chars", draft.len());
            updates.insert("draft".to_string(), json!(draft));
            updates
        });

    let editor_node = AgentNode::new(editor)
        .with_input_mapper(|state| {
            let draft = state.get("draft").and_then(|v| v.as_str()).unwrap_or("");
            Content::new("user").with_text(&format!("Edit and polish this draft:\n{}", draft))
        })
        .with_output_mapper(|events| {
            let mut updates = HashMap::new();
            let mut final_text = String::new();
            for event in events {
                if let Some(content) = event.content() {
                    for part in &content.parts {
                        if let Some(text) = part.text() { final_text.push_str(text); }
                    }
                }
            }
            println!("  📝 Edited: {} chars", final_text.len());
            updates.insert("final_article".to_string(), json!(final_text));
            updates
        });

    let graph = StateGraph::with_channels(&[
        "topic", "audience", "max_words",
        "research", "brief", "draft",
        "word_count", "reading_time", "needs_edit",
        "final_article",
    ])
    // Step 1: SET — Load config into state
    .add_node_fn("load_config", |ctx| async move {
        let topic = ctx.get("topic").and_then(|v| v.as_str()).unwrap_or("Rust").to_string();
        let audience = ctx.get("audience").and_then(|v| v.as_str()).unwrap_or("developers").to_string();
        println!("  📦 SET: topic={}, audience={}", topic, audience);
        Ok(NodeOutput::new()
            .with_update("topic", json!(topic))
            .with_update("audience", json!(audience))
            .with_update("max_words", json!(200)))
    })
    // Step 2: LLM researches the topic
    .add_node(researcher_node)
    // Step 3: TRANSFORM — Format research into a structured brief
    .add_node_fn("format_brief", |ctx| async move {
        let topic = ctx.get("topic").and_then(|v| v.as_str()).unwrap_or("");
        let audience = ctx.get("audience").and_then(|v| v.as_str()).unwrap_or("");
        let research = ctx.get("research").and_then(|v| v.as_str()).unwrap_or("");
        let brief = format!(
            "Topic: {}\nAudience: {}\nTone: informative, engaging\n\nKey Points:\n{}",
            topic, audience, research
        );
        println!("  🔄 TRANSFORM: Research → structured brief");
        Ok(NodeOutput::new().with_update("brief", json!(brief)))
    })
    // Step 4: LLM writes the article
    .add_node(writer_node)
    // Step 5: TRANSFORM — Calculate metadata
    .add_node_fn("add_metadata", |ctx| async move {
        let draft = ctx.get("draft").and_then(|v| v.as_str()).unwrap_or("");
        let word_count = draft.split_whitespace().count();
        let reading_time = (word_count as f64 / 200.0).ceil() as u64; // ~200 wpm
        let needs_edit = word_count < 50; // too short = needs editing
        println!("  🔄 TRANSFORM: {} words, ~{} min read, needs_edit={}", word_count, reading_time, needs_edit);
        Ok(NodeOutput::new()
            .with_update("word_count", json!(word_count))
            .with_update("reading_time", json!(reading_time))
            .with_update("needs_edit", json!(needs_edit)))
    })
    // Step 6: SWITCH — Route based on quality check
    .add_node(editor_node)
    .add_node_fn("publish", |ctx| async move {
        let draft = ctx.get("draft").and_then(|v| v.as_str()).unwrap_or("");
        println!("  🚀 PUBLISH: Article ready (skipped editing)");
        Ok(NodeOutput::new().with_update("final_article", json!(draft)))
    })
    // Wire it up
    .add_edge(START, "load_config")
    .add_edge("load_config", "researcher")
    .add_edge("researcher", "format_brief")
    .add_edge("format_brief", "writer")
    .add_edge("writer", "add_metadata")
    // SWITCH: if needs editing → editor, otherwise → publish directly
    .add_conditional_edges(
        "add_metadata",
        |state| {
            let needs_edit = state.get("needs_edit").and_then(|v| v.as_bool()).unwrap_or(true);
            if needs_edit { "editor".to_string() } else { "publish".to_string() }
        },
        [("editor", "editor"), ("publish", "publish")],
    )
    .add_edge("editor", END)
    .add_edge("publish", END)
    .compile()?;

    let mut input = HashMap::new();
    input.insert("topic".to_string(), json!("Why Rust is great for building AI agents"));
    input.insert("audience".to_string(), json!("software engineers"));

    let result = graph.invoke(input, ExecutionConfig::new("content-1")).await?;

    println!("\n=== Published Article ===");
    println!("Words: {} | Reading time: ~{} min",
        result.get("word_count").and_then(|v| v.as_u64()).unwrap_or(0),
        result.get("reading_time").and_then(|v| v.as_u64()).unwrap_or(0));
    println!("{}", result.get("final_article")
        .or(result.get("draft"))
        .and_then(|v| v.as_str())
        .unwrap_or("No article"));

    println!("\n=== Pipeline Complete ===");
    println!("• SET loaded config (deterministic, free)");
    println!("• TRANSFORM formatted data between agents (deterministic, free)");
    println!("• SWITCH routed based on quality threshold (deterministic, free)");
    println!("• 3 LLM agents handled research, writing, and editing");
    Ok(())
}
