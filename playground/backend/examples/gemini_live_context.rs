// Gemini Live — Context Mutation via Session Resumption
//
// Demonstrates mid-session context changes with Gemini Live.
// Unlike OpenAI (which uses session.update), Gemini uses session
// resumption — the runner reconnects with new config transparently.
//
// Phase 1: Tech support agent with account lookup tool
// Phase 2: Switch to billing agent with invoice tool
// Same API as OpenAI — RealtimeRunner handles the difference.
//
// Requires: GOOGLE_API_KEY

use adk_realtime::config::{RealtimeConfig, SessionUpdateConfig, ToolDefinition};
use adk_realtime::events::ServerEvent;
use adk_realtime::runner::{FnToolHandler, RealtimeRunner};
use serde_json::json;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let api_key = std::env::var("GOOGLE_API_KEY")?;

    println!("=== Gemini Live — Context Mutation (Session Resumption) ===\n");

    let backend = adk_realtime::gemini::GeminiLiveBackend::studio(&api_key);
    let model = adk_realtime::gemini::GeminiRealtimeModel::new(
        backend,
        "gemini-2.0-flash-live-001",
    );

    // ── Phase 1: Tech support agent ──
    let lookup_tool = ToolDefinition::new("lookup_account")
        .with_description("Look up a customer account by ID")
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "account_id": { "type": "string", "description": "Customer account ID" }
            },
            "required": ["account_id"]
        }));

    let runner = RealtimeRunner::builder()
        .model(Arc::new(model))
        .config(
            RealtimeConfig::default()
                .with_instruction(
                    "You are a technical support agent. Help users troubleshoot issues. \
                     Use the lookup_account tool when they mention an account. Be concise — 2-3 sentences.",
                )
                .with_modalities(vec!["text".to_string()]),
        )
        .tool(
            lookup_tool,
            FnToolHandler::new(|call| {
                let id = call.arguments["account_id"].as_str().unwrap_or("?");
                println!("  🔍 lookup_account(\"{}\")", id);
                Ok(json!({
                    "account_id": id,
                    "plan": "premium",
                    "status": "active",
                    "open_tickets": 2,
                    "last_login": "2026-03-28"
                }))
            }),
        )
        .build()?;

    println!("📡 Connecting to Gemini Live...");
    runner.connect().await?;
    println!("✅ Connected — Phase 1: Tech Support\n");

    // Ask about an account
    let q1 = "I'm having trouble with my account ABC-123, can you look it up?";
    println!("👤 User: {}\n", q1);
    runner.send_text(q1).await?;
    runner.create_response().await?;

    let r1 = collect_response(&runner).await;
    println!("🤖 Support: {}\n", r1);

    // ── Phase 2: Switch to billing agent ──
    println!("── Switching to billing agent (session resumption) ──\n");

    let invoice_tool = ToolDefinition::new("get_invoice")
        .with_description("Retrieve an invoice by number")
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "invoice_number": { "type": "string" }
            },
            "required": ["invoice_number"]
        }));

    let update = SessionUpdateConfig(
        RealtimeConfig::default()
            .with_instruction(
                "You are a billing specialist. Help users with invoices and payments. \
                 Use the get_invoice tool when they ask about an invoice. Be precise with numbers. 2-3 sentences.",
            )
            .with_tools(vec![invoice_tool]),
    );

    match runner.update_session(update).await {
        Ok(()) => println!("✅ Context updated — Phase 2: Billing Agent\n"),
        Err(e) => println!("⚠️ Context update: {} — continuing with original config\n", e),
    }

    // Ask about an invoice
    let q2 = "Can you pull up invoice INV-2026-0042?";
    println!("👤 User: {}\n", q2);
    runner.send_text(q2).await?;
    runner.create_response().await?;

    let r2 = collect_response(&runner).await;
    println!("🤖 Billing: {}\n", r2);

    runner.close().await?;

    println!("=== Context Mutation Features ===");
    println!("• Gemini uses session resumption (reconnect with new config)");
    println!("• OpenAI uses in-place session.update — same API via RealtimeRunner");
    println!("• Tools are swapped atomically (lookup_account → get_invoice)");
    println!("• Persona + instructions change without dropping conversation context");
    println!("• RealtimeRunner abstracts the provider difference");
    Ok(())
}

async fn collect_response(runner: &RealtimeRunner) -> String {
    let mut text = String::new();
    let mut count = 0;
    while let Some(event) = runner.next_event().await {
        count += 1;
        match event {
            Ok(ServerEvent::TextDelta { delta, .. }) => text.push_str(&delta),
            Ok(ServerEvent::TranscriptDelta { delta, .. }) => text.push_str(&delta),
            Ok(ServerEvent::ResponseDone { .. }) => break,
            Ok(ServerEvent::FunctionCallDone { .. }) => {}
            Ok(ServerEvent::Error { error, .. }) => {
                text = format!("Error: {}", error.message);
                break;
            }
            Err(e) => {
                text = format!("Stream error: {}", e);
                break;
            }
            _ => {}
        }
        if count > 300 { break; }
    }
    text
}
