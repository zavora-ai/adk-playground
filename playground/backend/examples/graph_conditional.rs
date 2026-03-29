use adk_rust::graph::{AgentNode, ExecutionConfig};
use adk_rust::graph::{NodeOutput, StateGraph, END, START};
use adk_rust::prelude::*;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

// ── Graph: LLM Conditional Routing ──
// An LLM classifier agent reads a support message and decides the priority.
// Conditional edges route to specialist LLM agents based on the classification.
// Each specialist generates a tailored response for their priority level.
// Shows: AgentNode + conditional_edges + multi-agent routing in a graph.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key = std::env::var("GOOGLE_API_KEY")?;
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-2.5-flash")?);

    println!("=== Graph: LLM Conditional Routing ===\n");

    // ── Classifier agent: decides priority ──
    let classifier = Arc::new(
        LlmAgentBuilder::new("classifier")
            .model(model.clone())
            .instruction(
                "You are a support ticket classifier. Read the message and reply with \
                 ONLY one word: 'high', 'medium', or 'low'.\n\
                 - high: outages, security, data loss, urgent\n\
                 - medium: bugs, help requests, account issues\n\
                 - low: feature requests, general questions, feedback",
            )
            .build()?,
    );

    // ── Specialist agents for each priority ──
    let urgent_agent = Arc::new(
        LlmAgentBuilder::new("urgent_responder")
            .model(model.clone())
            .instruction(
                "You are an urgent incident responder. Acknowledge the severity, \
                 provide immediate next steps, and set expectations. \
                 Be direct and reassuring. 2-3 sentences max.",
            )
            .build()?,
    );

    let support_agent = Arc::new(
        LlmAgentBuilder::new("support_agent")
            .model(model.clone())
            .instruction(
                "You are a helpful support agent. Create a ticket, ask clarifying \
                 questions if needed, and provide a timeline. 2-3 sentences max.",
            )
            .build()?,
    );

    let info_agent = Arc::new(
        LlmAgentBuilder::new("info_agent")
            .model(model)
            .instruction(
                "You are a friendly info desk agent. Answer the question or \
                 acknowledge the feedback warmly. 1-2 sentences max.",
            )
            .build()?,
    );

    // ── Build graph nodes ──
    let classifier_node = AgentNode::new(classifier)
        .with_input_mapper(|state| {
            let msg = state.get("message").and_then(|v| v.as_str()).unwrap_or("");
            Content::new("user").with_text(format!("Classify this support message:\n\n{}", msg))
        })
        .with_output_mapper(|events| {
            let mut updates = HashMap::new();
            let mut text = String::new();
            for event in events {
                if let Some(c) = event.content() {
                    for p in &c.parts { if let Some(t) = p.text() { text.push_str(t); } }
                }
            }
            let lower = text.to_lowercase().trim().to_string();
            let priority = if lower.contains("high") { "high" }
                else if lower.contains("medium") { "medium" }
                else { "low" };
            println!("🏷️  Classifier → {}", priority);
            updates.insert("priority".to_string(), json!(priority));
            updates
        });

    let make_responder = |agent: Arc<dyn Agent + Send + Sync>, emoji: &'static str| {
        AgentNode::new(agent)
            .with_input_mapper(|state| {
                let msg = state.get("message").and_then(|v| v.as_str()).unwrap_or("");
                Content::new("user").with_text(format!("Respond to this support message:\n\n{}", msg))
            })
            .with_output_mapper(move |events| {
                let mut updates = HashMap::new();
                let mut response = String::new();
                for event in events {
                    if let Some(c) = event.content() {
                        for p in &c.parts { if let Some(t) = p.text() { response.push_str(t); } }
                    }
                }
                println!("{} Response: {}", emoji, &response[..response.len().min(120)]);
                updates.insert("response".to_string(), json!(response));
                updates
            })
    };

    let urgent_node = make_responder(urgent_agent, "🚨");
    let support_node = make_responder(support_agent, "📋");
    let info_node = make_responder(info_agent, "ℹ️");

    let graph = StateGraph::with_channels(&["message", "priority", "response"])
        .add_node(classifier_node)
        .add_node(urgent_node)
        .add_node(support_node)
        .add_node(info_node)
        .add_edge(START, "classifier")
        .add_conditional_edges(
            "classifier",
            |state| {
                state.get("priority")
                    .and_then(|v| v.as_str())
                    .unwrap_or("low")
                    .to_string()
            },
            [
                ("high", "urgent_responder"),
                ("medium", "support_agent"),
                ("low", "info_agent"),
            ],
        )
        .add_edge("urgent_responder", END)
        .add_edge("support_agent", END)
        .add_edge("info_agent", END)
        .compile()?;

    // ── Run three different messages ──
    let messages = [
        ("🔴", "URGENT: Our production database is down and we're losing customer data every minute!"),
        ("🟡", "I need help resetting my password — I've been locked out of my account for 2 days"),
        ("🟢", "Do you have plans to add a dark mode to the dashboard? Would love that feature"),
    ];

    for (icon, msg) in messages {
        println!("{} Input: \"{}\"\n", icon, &msg[..msg.len().min(70)]);
        let mut input = HashMap::new();
        input.insert("message".to_string(), json!(msg));
        let result = graph.invoke(input, ExecutionConfig::new("route")).await?;
        let priority = result.get("priority").and_then(|v| v.as_str()).unwrap_or("?");
        let response = result.get("response").and_then(|v| v.as_str()).unwrap_or("");
        println!("   Priority: {} | Response: {}\n", priority, response);
    }

    println!("=== Routing Summary ===");
    println!("• LLM classifier reads message → outputs priority tag");
    println!("• Conditional edges route to the right specialist agent");
    println!("• Each specialist has domain-specific instructions");
    println!("• No hardcoded keyword matching — the LLM understands context");
    Ok(())
}
