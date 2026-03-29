use adk_rust::graph::{AgentNode, ExecutionConfig};
use adk_rust::graph::{NodeOutput, StateGraph, END, START};
use adk_rust::prelude::*;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

// ── Action + Agent: Data Enrichment Pipeline ──
//
// Mixes deterministic action nodes with an LLM agent:
//   1. SET action — loads raw customer data into state
//   2. TRANSFORM action — normalizes and cleans the data
//   3. LLM Agent — analyzes the data and writes a personalized message
//
// Shows how action nodes handle boring data prep so the agent
// can focus on what LLMs are good at: reasoning and creativity.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key = std::env::var("GOOGLE_API_KEY")?;
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-3.1-flash-lite-preview")?);

    println!("=== Action + Agent: Data Enrichment Pipeline ===\n");

    // LLM agent that writes personalized outreach
    let writer = Arc::new(
        LlmAgentBuilder::new("outreach_writer")
            .model(model)
            .instruction(
                "You are a customer success writer. Given customer data in the state, \
                 write a short personalized outreach message (2-3 sentences). \
                 Reference their name, company, plan tier, and usage level. \
                 Be warm but professional.",
            )
            .build()?,
    );

    let writer_node = AgentNode::new(writer)
        .with_input_mapper(|state| {
            let name = state.get("name").and_then(|v| v.as_str()).unwrap_or("Customer");
            let company = state.get("company").and_then(|v| v.as_str()).unwrap_or("their company");
            let tier = state.get("tier").and_then(|v| v.as_str()).unwrap_or("standard");
            let usage = state.get("usage_level").and_then(|v| v.as_str()).unwrap_or("moderate");

            Content::new("user").with_text(&format!(
                "Write outreach for: {} at {} (plan: {}, usage: {})",
                name, company, tier, usage
            ))
        })
        .with_output_mapper(|events| {
            let mut updates = HashMap::new();
            let mut message = String::new();
            for event in events {
                if let Some(content) = event.content() {
                    for part in &content.parts {
                        if let Some(text) = part.text() {
                            message.push_str(text);
                        }
                    }
                }
            }
            println!("  ✉️  Agent wrote: {}", &message[..message.len().min(120)]);
            updates.insert("outreach_message".to_string(), json!(message));
            updates
        });

    let graph = StateGraph::with_channels(&[
        "raw_data", "name", "company", "tier", "usage_level", "outreach_message",
    ])
    // Step 1: SET — Extract and store fields from raw data
    .add_node_fn("set_fields", |ctx| async move {
        let raw = ctx.get("raw_data").cloned().unwrap_or(json!({}));
        let name = raw["name"].as_str().unwrap_or("Unknown").to_string();
        let company = raw["company"].as_str().unwrap_or("Unknown").to_string();
        let plan = raw["plan"].as_str().unwrap_or("free").to_string();
        let api_calls = raw["api_calls_30d"].as_u64().unwrap_or(0);

        println!("  📦 SET: Extracted {} at {} (plan: {})", name, company, plan);

        Ok(NodeOutput::new()
            .with_update("name", json!(name))
            .with_update("company", json!(company))
            .with_update("tier", json!(plan)))
    })
    // Step 2: TRANSFORM — Classify usage level from raw numbers
    .add_node_fn("classify_usage", |ctx| async move {
        let raw = ctx.get("raw_data").cloned().unwrap_or(json!({}));
        let api_calls = raw["api_calls_30d"].as_u64().unwrap_or(0);
        let tier = ctx.get("tier").and_then(|v| v.as_str()).unwrap_or("free");

        let usage_level = match (tier, api_calls) {
            (_, c) if c > 10000 => "power_user",
            (_, c) if c > 1000 => "active",
            ("enterprise", _) => "onboarding",
            _ => "light",
        };

        println!("  🔄 TRANSFORM: {} API calls → usage_level={}", api_calls, usage_level);

        Ok(NodeOutput::new().with_update("usage_level", json!(usage_level)))
    })
    // Step 3: LLM Agent — Write personalized outreach
    .add_node(writer_node)
    // Pipeline: SET → TRANSFORM → AGENT → END
    .add_edge(START, "set_fields")
    .add_edge("set_fields", "classify_usage")
    .add_edge("classify_usage", "outreach_writer")
    .add_edge("outreach_writer", END)
    .compile()?;

    // Run with sample customer data
    let mut input = HashMap::new();
    input.insert("raw_data".to_string(), json!({
        "name": "Sarah Chen",
        "company": "DataFlow Inc",
        "plan": "enterprise",
        "api_calls_30d": 15420,
        "signup_date": "2024-06-15"
    }));

    let result = graph.invoke(input, ExecutionConfig::new("enrich-1")).await?;

    println!("\n=== Pipeline Result ===");
    println!("Customer: {} at {}", 
        result.get("name").and_then(|v| v.as_str()).unwrap_or("?"),
        result.get("company").and_then(|v| v.as_str()).unwrap_or("?"));
    println!("Tier: {} | Usage: {}",
        result.get("tier").and_then(|v| v.as_str()).unwrap_or("?"),
        result.get("usage_level").and_then(|v| v.as_str()).unwrap_or("?"));
    println!("Message: {}", 
        result.get("outreach_message").and_then(|v| v.as_str()).unwrap_or("No message"));
    Ok(())
}
