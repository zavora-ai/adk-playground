use adk_rust::graph::{AgentNode, ExecutionConfig};
use adk_rust::graph::{NodeOutput, StateGraph, END, START};
use adk_rust::prelude::*;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

// ── Action + Agent: Smart Ticket Router ──
//
// Combines action nodes with LLM agents for intelligent routing:
//   1. LLM Agent — classifies the ticket (billing, technical, general)
//   2. SWITCH action — routes to the right handler based on classification
//   3. Specialist Agents — each handles their domain
//   4. LOOP pattern — supervisor can re-route if needed
//
// The SWITCH is deterministic (fast, cheap) while agents handle reasoning.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key = std::env::var("GOOGLE_API_KEY")?;
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-3.1-flash-lite-preview")?);

    println!("=== Action + Agent: Smart Ticket Router ===\n");

    // Classifier agent — determines ticket category
    let classifier = Arc::new(
        LlmAgentBuilder::new("classifier")
            .model(model.clone())
            .instruction(
                "Classify the support ticket into exactly one category. \
                 Reply with ONLY one word:\n\
                 - billing (payment, invoice, subscription, charge issues)\n\
                 - technical (bugs, errors, API, integration problems)\n\
                 - general (questions, feedback, feature requests)",
            )
            .build()?,
    );

    // Specialist agents
    let billing_agent = Arc::new(
        LlmAgentBuilder::new("billing_specialist")
            .model(model.clone())
            .instruction(
                "You are a billing specialist. Respond to the customer's billing issue \
                 with a helpful resolution. Be empathetic and specific. 2-3 sentences max.",
            )
            .build()?,
    );

    let tech_agent = Arc::new(
        LlmAgentBuilder::new("tech_specialist")
            .model(model.clone())
            .instruction(
                "You are a technical support engineer. Diagnose the issue and provide \
                 a clear fix or workaround. Include specific steps. 2-3 sentences max.",
            )
            .build()?,
    );

    let general_agent = Arc::new(
        LlmAgentBuilder::new("general_specialist")
            .model(model)
            .instruction(
                "You are a friendly support agent. Answer the customer's question \
                 or acknowledge their feedback warmly. 2-3 sentences max.",
            )
            .build()?,
    );

    // Classifier node — LLM decides the category
    let classifier_node = AgentNode::new(classifier)
        .with_input_mapper(|state| {
            let ticket = state.get("ticket").and_then(|v| v.as_str()).unwrap_or("");
            Content::new("user").with_text(&format!("Classify this ticket: {}", ticket))
        })
        .with_output_mapper(|events| {
            let mut updates = HashMap::new();
            for event in events {
                if let Some(content) = event.content() {
                    let text: String = content.parts.iter()
                        .filter_map(|p| p.text()).collect::<Vec<_>>().join("")
                        .to_lowercase().trim().to_string();
                    let category = if text.contains("billing") { "billing" }
                        else if text.contains("technical") { "technical" }
                        else { "general" };
                    println!("  🏷️  Classifier → {}", category);
                    updates.insert("category".to_string(), json!(category));
                }
            }
            updates
        });

    // Helper to build specialist nodes
    let make_specialist = |agent: Arc<dyn Agent + Send + Sync>, emoji: &'static str| {
        AgentNode::new(agent)
            .with_input_mapper(|state| {
                let ticket = state.get("ticket").and_then(|v| v.as_str()).unwrap_or("");
                Content::new("user").with_text(ticket)
            })
            .with_output_mapper(move |events| {
                let mut updates = HashMap::new();
                for event in events {
                    if let Some(content) = event.content() {
                        let text: String = content.parts.iter()
                            .filter_map(|p| p.text()).collect::<Vec<_>>().join("");
                        println!("  {} Response: {}", emoji, &text[..text.len().min(120)]);
                        updates.insert("response".to_string(), json!(text));
                    }
                }
                updates
            })
    };

    let graph = StateGraph::with_channels(&["ticket", "category", "response"])
        // Step 1: LLM classifies the ticket
        .add_node(classifier_node)
        // Step 2: SWITCH — deterministic routing (no LLM cost!)
        // This is the action node pattern: fast, cheap conditional routing
        .add_node(make_specialist(billing_agent, "💳"))
        .add_node(make_specialist(tech_agent, "🔧"))
        .add_node(make_specialist(general_agent, "💬"))
        .add_edge(START, "classifier")
        .add_conditional_edges(
            "classifier",
            |state| {
                // Deterministic switch — like an action Switch node
                state.get("category")
                    .and_then(|v| v.as_str())
                    .unwrap_or("general")
                    .to_string()
            },
            [
                ("billing", "billing_specialist"),
                ("technical", "tech_specialist"),
                ("general", "general_specialist"),
            ],
        )
        .add_edge("billing_specialist", END)
        .add_edge("tech_specialist", END)
        .add_edge("general_specialist", END)
        .compile()?;

    // Test with different ticket types
    let tickets = [
        "I was charged twice for my Pro subscription last month. Order #12345.",
        "The /api/v2/users endpoint returns 500 errors when I include pagination params.",
        "Do you have plans to add dark mode to the dashboard? Love the product!",
    ];

    for ticket in tickets {
        println!("📩 Ticket: \"{}\"\n", &ticket[..ticket.len().min(70)]);
        let mut input = HashMap::new();
        input.insert("ticket".to_string(), json!(ticket));
        let result = graph.invoke(input, ExecutionConfig::new("ticket")).await?;
        println!("  ✅ Category: {} | Resolved\n",
            result.get("category").and_then(|v| v.as_str()).unwrap_or("?"));
    }

    println!("=== All tickets routed and resolved ===");
    println!("• LLM classifier picks the category (smart)");
    println!("• Conditional edges route deterministically (fast, free)");
    println!("• Specialist agents handle domain-specific responses");
    Ok(())
}
